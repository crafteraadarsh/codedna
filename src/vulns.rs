//! Known vulnerabilities in dependencies, from the OSV database.
//!
//! Lockfiles give the exact version of every dependency. Those versions are
//! checked against <https://osv.dev>, which collects advisories from GitHub,
//! RustSec, PyPI, the Go team and others. This is the only part of the audit
//! that uses the network, so it only runs when asked for: package names and
//! versions are sent to osv.dev, nothing else.
//!
//! Read: `Cargo.lock`, `package-lock.json`, `npm-shrinkwrap.json`,
//! `yarn.lock`, `pnpm-lock.yaml`, `requirements*.txt` (pinned `==` lines),
//! `poetry.lock`, `uv.lock`, `Pipfile.lock`, `go.mod`, `Gemfile.lock` and
//! `composer.lock`.

use crate::architecture::Confidence;
use crate::audit::{Finding, Severity, VULNERABLE_DEPENDENCY};
use crate::safe_text::sanitize;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

const OSV_API: &str = "https://api.osv.dev/v1";
/// Packages per batch query; OSV accepts up to 1000.
const BATCH: usize = 1000;
/// Most packages checked, so a huge monorepo cannot run for minutes.
const MAX_PACKAGES: usize = 20_000;
/// Most advisories looked up in detail; the rest count as medium.
const MAX_DETAILS: usize = 300;
/// Advisory ids named in one finding before "and N more".
const IDS_SHOWN: usize = 5;
/// Lockfiles bigger than this are skipped.
const MAX_LOCKFILE: u64 = 50 * 1024 * 1024;

/// A dependency at an exact version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Package {
    /// OSV ecosystem name, e.g. `npm` or `crates.io`.
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
    /// The lockfile it came from, relative to the audited folder.
    pub file: String,
}

/// The result of a dependency check.
#[derive(Debug, Clone, Default)]
pub struct DependencyCheck {
    /// Distinct packages checked; 0 when the database could not be reached.
    pub packages: usize,
    /// Lockfiles read.
    pub lockfiles: Vec<String>,
    pub findings: Vec<Finding>,
    /// Set when the database could not be reached; `findings` is then empty.
    pub error: Option<String>,
}

/// Where advisories come from. The real one talks to osv.dev; tests use a
/// fake.
pub trait Source: Sync {
    fn post(&self, path: &str, body: &Value) -> Result<Value, String>;
    fn get(&self, path: &str) -> Result<Value, String>;
}

/// The public OSV API.
pub struct OsvApi {
    agent: ureq::Agent,
}

impl OsvApi {
    pub fn new() -> OsvApi {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("codedna/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        OsvApi { agent }
    }
}

impl Default for OsvApi {
    fn default() -> Self {
        OsvApi::new()
    }
}

impl Source for OsvApi {
    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.agent
            .post(format!("{OSV_API}{path}"))
            .send_json(body)
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_json()
            .map_err(|e| e.to_string())
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.agent
            .get(format!("{OSV_API}{path}"))
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_json()
            .map_err(|e| e.to_string())
    }
}

/// Read the lockfiles under `root` and check every dependency against OSV.
pub fn check(root: &Path, source: &dyn Source) -> DependencyCheck {
    let (packages, lockfiles) = collect(root);
    let mut result = DependencyCheck {
        packages: packages.len(),
        lockfiles,
        ..Default::default()
    };
    match query(&packages, source) {
        Ok(findings) => result.findings = findings,
        Err(e) => {
            result.packages = 0;
            result.error = Some(format!(
                "Could not reach the vulnerability database (osv.dev): {}",
                sanitize(&e)
            ))
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Lockfiles
// ---------------------------------------------------------------------------

/// Every distinct package found in lockfiles under `root`, and the lockfiles
/// read.
pub fn collect(root: &Path) -> (Vec<Package>, Vec<String>) {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some("node_modules" | "vendor" | "target" | ".git" | ".venv" | "venv")
            )
        })
        .build();
    let mut lockfiles = Vec::new();
    let mut packages = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        let Some(parse) = parser_for(name) else {
            continue;
        };
        if entry.metadata().map_or(true, |m| m.len() > MAX_LOCKFILE) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let relative = sanitize(&relative.to_string_lossy().replace('\\', "/"));
        let found = parse(&text);
        if found.is_empty() {
            continue;
        }
        packages.extend(found.into_iter().map(|(ecosystem, name, version)| Package {
            ecosystem,
            name,
            version,
            file: relative.clone(),
        }));
        lockfiles.push(relative);
    }
    lockfiles.sort();
    packages.sort();
    packages
        .dedup_by(|a, b| a.ecosystem == b.ecosystem && a.name == b.name && a.version == b.version);
    packages.truncate(MAX_PACKAGES);
    (packages, lockfiles)
}

type Found = Vec<(&'static str, String, String)>;

fn parser_for(file_name: &str) -> Option<fn(&str) -> Found> {
    Some(match file_name {
        "Cargo.lock" => cargo_lock,
        "package-lock.json" | "npm-shrinkwrap.json" => package_lock,
        "yarn.lock" => yarn_lock,
        "pnpm-lock.yaml" => pnpm_lock,
        "poetry.lock" | "uv.lock" => python_toml_lock,
        "Pipfile.lock" => pipfile_lock,
        "go.mod" => go_mod,
        "Gemfile.lock" => gemfile_lock,
        "composer.lock" => composer_lock,
        n if n.starts_with("requirements") && n.ends_with(".txt") => requirements_txt,
        _ => return None,
    })
}

fn cargo_lock(text: &str) -> Found {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    toml_packages(&doc)
        .filter(|p| {
            // Path and Git dependencies are not on crates.io.
            p.get("source")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s.starts_with("registry+") || s.starts_with("sparse+"))
        })
        .filter_map(|p| toml_name_version(p, "crates.io"))
        .collect()
}

fn python_toml_lock(text: &str) -> Found {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    toml_packages(&doc)
        .filter(|p| {
            // uv marks the project itself and local packages this way.
            !p.get("source").and_then(|s| s.as_table()).is_some_and(|s| {
                ["virtual", "editable", "path", "directory", "git"]
                    .iter()
                    .any(|k| s.contains_key(*k))
            })
        })
        .filter_map(|p| toml_name_version(p, "PyPI"))
        .collect()
}

fn toml_packages(doc: &toml::Table) -> impl Iterator<Item = &toml::Table> {
    doc.get("package")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_table())
}

fn toml_name_version(
    p: &toml::Table,
    ecosystem: &'static str,
) -> Option<(&'static str, String, String)> {
    let name = p.get("name")?.as_str()?;
    let version = p.get("version")?.as_str()?;
    Some((ecosystem, name.to_string(), version.to_string()))
}

fn package_lock(text: &str) -> Found {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(packages) = doc.get("packages").and_then(Value::as_object) {
        // Lockfile v2 and v3: keys are install paths.
        for (path, p) in packages {
            let Some(at) = path.rfind("node_modules/") else {
                continue;
            };
            if p.get("link").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let name = p
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&path[at + "node_modules/".len()..]);
            if let Some(version) = p.get("version").and_then(Value::as_str) {
                out.push(("npm", name.to_string(), version.to_string()));
            }
        }
    } else if let Some(deps) = doc.get("dependencies") {
        // Lockfile v1: nested `dependencies`.
        let mut stack = vec![deps];
        while let Some(deps) = stack.pop() {
            for (name, d) in deps.as_object().into_iter().flatten() {
                if let Some(version) = d.get("version").and_then(Value::as_str) {
                    if is_plain_version(version) {
                        out.push(("npm", name.clone(), version.to_string()));
                    }
                }
                if let Some(nested) = d.get("dependencies") {
                    stack.push(nested);
                }
            }
        }
    }
    out
}

/// Yarn 1 (`version "1.2.3"`) and Yarn 2+ (`version: 1.2.3`).
fn yarn_lock(text: &str) -> Found {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        if !line.starts_with(' ') && line.ends_with(':') && !line.starts_with('#') {
            // `"@scope/a@^1.0.0", "@scope/a@^1.1.0":` or `a@npm:^1.0.0:`
            let first = line.trim_end_matches(':').split(',').next().unwrap_or("");
            let spec = first.trim().trim_matches('"');
            name = spec
                .get(1..)
                .and_then(|rest| rest.find('@'))
                .map(|at| spec[..at + 1].to_string())
                .filter(|_| {
                    // Workspaces, links and patches are not from the registry.
                    ![
                        "@workspace:",
                        "@link:",
                        "@portal:",
                        "@file:",
                        "@patch:",
                        "@exec:",
                    ]
                    .iter()
                    .any(|p| spec.contains(p))
                });
            continue;
        }
        let trimmed = line.trim();
        let version = trimmed
            .strip_prefix("version ")
            .or_else(|| trimmed.strip_prefix("version: "))
            .map(|v| v.trim().trim_matches('"'));
        if let (Some(n), Some(v)) = (&name, version) {
            if line.starts_with("  ") && !line.starts_with("    ") && is_plain_version(v) {
                out.push(("npm", n.clone(), v.to_string()));
            }
        }
    }
    out
}

/// pnpm 5 (`/a/1.2.3:`), 6 (`/a@1.2.3:`) and 9 (`a@1.2.3:`).
fn pnpm_lock(text: &str) -> Found {
    let mut out = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            in_packages = line == "packages:";
            continue;
        }
        if !in_packages || !line.starts_with("  ") || line.starts_with("   ") {
            continue;
        }
        let Some(key) = line.trim().strip_suffix(':') else {
            continue;
        };
        let key = key.trim_matches(|c| c == '\'' || c == '"');
        let key = key.strip_prefix('/').unwrap_or(key);
        // Drop peer suffixes: `a@1.0.0(react@18.2.0)` or `/a/1.0.0_react@18.2.0`.
        let key = key.split('(').next().unwrap_or(key);
        // pnpm 5 separates the version with `/`: `@scope/a/1.0.0`.
        let v5 = key.rfind('/').filter(|&at| {
            key[at + 1..].starts_with(|c: char| c.is_ascii_digit()) && !key[1..at].contains('@')
        });
        let split = match v5 {
            Some(at) => Some((&key[..at], &key[at + 1..])),
            None => key
                .get(1..)
                .and_then(|rest| rest.rfind('@'))
                .map(|at| (&key[..at + 1], &key[at + 2..])),
        };
        let Some((name, version)) = split else {
            continue;
        };
        let version = version.split('_').next().unwrap_or(version);
        if !name.is_empty() && is_plain_version(version) {
            out.push(("npm", name.to_string(), version.to_string()));
        }
    }
    out
}

fn requirements_txt(text: &str) -> Found {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next()?.split(';').next()?.trim();
            let (name, version) = line.split_once("==")?;
            let name = name.split('[').next()?.trim();
            let version = version.split(',').next()?.trim();
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                && is_plain_version(version);
            valid.then(|| ("PyPI", name.to_string(), version.to_string()))
        })
        .collect()
}

fn pipfile_lock(text: &str) -> Found {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    ["default", "develop"]
        .iter()
        .filter_map(|section| doc.get(section)?.as_object())
        .flatten()
        .filter_map(|(name, p)| {
            let version = p.get("version")?.as_str()?.strip_prefix("==")?;
            Some(("PyPI", name.clone(), version.to_string()))
        })
        .collect()
}

fn go_mod(text: &str) -> Found {
    let mut out = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let spec = if in_require {
            if line == ")" {
                in_require = false;
                continue;
            }
            line
        } else if line == "require (" {
            in_require = true;
            continue;
        } else if let Some(rest) = line.strip_prefix("require ") {
            rest
        } else {
            continue;
        };
        let mut parts = spec.split_whitespace();
        if let (Some(module), Some(version)) = (parts.next(), parts.next()) {
            // OSV lists Go versions without the leading `v`.
            let version = version.strip_prefix('v').unwrap_or(version);
            if is_plain_version(version) {
                out.push(("Go", module.to_string(), version.to_string()));
            }
        }
    }
    out
}

fn gemfile_lock(text: &str) -> Found {
    let mut out = Vec::new();
    let mut in_gems = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            in_gems = line == "GEM";
            continue;
        }
        // Gems are at four spaces; their own dependencies at six.
        if !in_gems || !line.starts_with("    ") || line.starts_with("     ") {
            continue;
        }
        let Some((name, rest)) = line.trim().split_once(" (") else {
            continue;
        };
        let version = rest.trim_end_matches(')');
        // Drop a platform suffix: `1.16.0-x86_64-linux`.
        let version = version.split('-').next().unwrap_or(version);
        if is_plain_version(version) {
            out.push(("RubyGems", name.to_string(), version.to_string()));
        }
    }
    out
}

fn composer_lock(text: &str) -> Found {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|section| doc.get(section)?.as_array())
        .flatten()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?;
            let version = p.get("version")?.as_str()?;
            let version = version.strip_prefix('v').unwrap_or(version);
            is_plain_version(version).then(|| ("Packagist", name.to_string(), version.to_string()))
        })
        .collect()
}

/// A released version such as `1.2.3` or `2.0.0-rc.1`, not a range, path or
/// Git reference.
fn is_plain_version(v: &str) -> bool {
    v.starts_with(|c: char| c.is_ascii_digit())
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
}

// ---------------------------------------------------------------------------
// OSV
// ---------------------------------------------------------------------------

fn query(packages: &[Package], source: &dyn Source) -> Result<Vec<Finding>, String> {
    // Advisory ids per package.
    let mut hits: Vec<(&Package, Vec<String>)> = Vec::new();
    for chunk in packages.chunks(BATCH) {
        let queries: Vec<Value> = chunk
            .iter()
            .map(|p| json!({"package": {"name": p.name, "ecosystem": p.ecosystem}, "version": p.version}))
            .collect();
        let response = source.post("/querybatch", &json!({ "queries": queries }))?;
        let results = response
            .get("results")
            .and_then(Value::as_array)
            .ok_or("unexpected response")?;
        for (package, result) in chunk.iter().zip(results) {
            let ids: Vec<String> = result
                .get("vulns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.get("id")?.as_str().map(String::from))
                .collect();
            if !ids.is_empty() {
                hits.push((package, ids));
            }
        }
    }

    let mut ids: Vec<&String> = hits.iter().flat_map(|(_, ids)| ids).collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_DETAILS);
    let details: HashMap<&String, Advisory> = ids
        .par_iter()
        .filter_map(|id| {
            let record = source.get(&format!("/vulns/{id}")).ok()?;
            Some((*id, Advisory::from_osv(&record)))
        })
        .collect();

    Ok(hits
        .iter()
        .filter_map(|(package, ids)| finding(package, ids, &details))
        .collect())
}

/// What a finding needs from one advisory.
#[derive(Debug, Clone, PartialEq)]
struct Advisory {
    summary: String,
    severity: Severity,
    withdrawn: bool,
    /// Fixed versions per affected package, keyed by `ecosystem/name`.
    fixed: BTreeMap<String, Vec<String>>,
}

impl Advisory {
    fn from_osv(v: &Value) -> Advisory {
        let summary = v
            .get("summary")
            .and_then(Value::as_str)
            .or_else(|| {
                v.get("details")
                    .and_then(Value::as_str)
                    .and_then(|d| d.lines().next())
            })
            .unwrap_or("")
            .trim()
            .to_string();
        let mut fixed: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut informational = false;
        for affected in v
            .get("affected")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let pkg = affected.get("package");
            let key = format!(
                "{}/{}",
                pkg.and_then(|p| p.get("ecosystem"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                pkg.and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
            if affected
                .pointer("/database_specific/informational")
                .is_some_and(|i| i.is_string())
            {
                informational = true;
            }
            let versions = affected
                .get("ranges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|r| r.get("type").and_then(Value::as_str) != Some("GIT"))
                .flat_map(|r| {
                    r.get("events")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|e| e.get("fixed")?.as_str().map(String::from));
            fixed.entry(key).or_default().extend(versions);
        }
        let severity = if informational {
            // RustSec notices such as "unmaintained".
            Severity::Low
        } else if let Some(level) = v
            .pointer("/database_specific/severity")
            .and_then(Value::as_str)
        {
            match level.to_ascii_uppercase().as_str() {
                "CRITICAL" | "HIGH" => Severity::High,
                "LOW" => Severity::Low,
                _ => Severity::Medium,
            }
        } else {
            v.get("severity")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|s| s.get("type").and_then(Value::as_str) == Some("CVSS_V3"))
                .filter_map(|s| cvss3_score(s.get("score")?.as_str()?))
                .map(severity_of_score)
                .max()
                .unwrap_or(Severity::Medium)
        };
        Advisory {
            summary,
            severity,
            withdrawn: v.get("withdrawn").is_some_and(|w| !w.is_null()),
            fixed,
        }
    }
}

fn finding(
    package: &Package,
    ids: &[String],
    details: &HashMap<&String, Advisory>,
) -> Option<Finding> {
    let key = format!("{}/{}", package.ecosystem, package.name);
    let known: Vec<(&String, Option<&Advisory>)> = ids
        .iter()
        .map(|id| (id, details.get(id)))
        .filter(|(_, a)| !a.is_some_and(|a| a.withdrawn))
        .collect();
    if known.is_empty() {
        return None;
    }
    let severity = known
        .iter()
        .map(|(_, a)| a.map_or(Severity::Medium, |a| a.severity))
        .max()?;
    // The worst advisory first, so its summary leads the message.
    let mut ordered = known.clone();
    ordered.sort_by(|a, b| {
        let sev = |x: &(&String, Option<&Advisory>)| x.1.map_or(Severity::Medium, |a| a.severity);
        sev(b).cmp(&sev(a)).then_with(|| a.0.cmp(b.0))
    });
    let fixed: Vec<&String> = known
        .iter()
        .filter_map(|(_, a)| a.and_then(|a| a.fixed.get(&key)))
        .flatten()
        .filter(|v| compare_versions(v, &package.version).is_gt())
        .collect();
    let upgrade = fixed.into_iter().max_by(|a, b| compare_versions(a, b));

    let count = known.len();
    let mut message = format!(
        "{} {} ({}) has {} known vulnerabilit{}",
        sanitize(&package.name),
        sanitize(&package.version),
        package.ecosystem,
        count,
        if count == 1 { "y" } else { "ies" }
    );
    let lead = ordered[0].1.map(|a| a.summary.as_str()).unwrap_or("");
    if !lead.is_empty() {
        message.push_str(&format!(", e.g. \"{}\"", sanitize(&shorten(lead, 120))));
    }
    let shown: Vec<String> = ordered
        .iter()
        .take(IDS_SHOWN)
        .map(|(id, _)| sanitize(id))
        .collect();
    message.push_str(&format!(": {}", shown.join(", ")));
    if count > IDS_SHOWN {
        message.push_str(&format!(" and {} more", count - IDS_SHOWN));
    }
    match upgrade {
        Some(v) => message.push_str(&format!(". Fixed in {}.", sanitize(v))),
        None => message.push_str(". No fixed version listed."),
    }

    Some(Finding {
        severity,
        // The version matches a published advisory; whether the vulnerable
        // code is reachable from this project is another question.
        confidence: Confidence::High,
        check: VULNERABLE_DEPENDENCY,
        file: package.file.clone(),
        line: None,
        message,
        context: None,
    })
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Compare dotted versions numerically where both parts are numbers, so
/// `1.10.0` sorts after `1.9.0`. Good enough to pick the newest fix.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<String> { v.split(['.', '-', '+']).map(String::from).collect() };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).map(String::as_str).unwrap_or("0");
        let y = pb.get(i).map(String::as_str).unwrap_or("0");
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if ord.is_ne() {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

/// CVSS 3.x base score from a vector such as
/// `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H`.
fn cvss3_score(vector: &str) -> Option<f64> {
    let m: HashMap<&str, &str> = vector
        .split('/')
        .skip(1)
        .filter_map(|part| part.split_once(':'))
        .collect();
    let changed = *m.get("S")? == "C";
    let av = match *m.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.2,
        _ => return None,
    };
    let ac = match *m.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let pr = match (*m.get("PR")?, changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.5,
        _ => return None,
    };
    let ui = match *m.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let cia = |k: &str| -> Option<f64> {
        match *m.get(k)? {
            "H" => Some(0.56),
            "L" => Some(0.22),
            "N" => Some(0.0),
            _ => None,
        }
    };
    let iss = 1.0 - (1.0 - cia("C")?) * (1.0 - cia("I")?) * (1.0 - cia("A")?);
    let impact = if changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15)
    } else {
        6.42 * iss
    };
    if impact <= 0.0 {
        return Some(0.0);
    }
    let exploitability = 8.22 * av * ac * pr * ui;
    let total = if changed {
        1.08 * (impact + exploitability)
    } else {
        impact + exploitability
    };
    Some(round_up(total.min(10.0)))
}

/// Round up to one decimal, as the CVSS 3.1 specification defines it.
fn round_up(x: f64) -> f64 {
    let i = (x * 100_000.0).round() as i64;
    if i % 10_000 == 0 {
        i as f64 / 100_000.0
    } else {
        ((i / 10_000) + 1) as f64 / 10.0
    }
}

fn severity_of_score(score: f64) -> Severity {
    if score >= 7.0 {
        Severity::High
    } else if score >= 4.0 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    fn names(found: Found) -> Vec<String> {
        found
            .into_iter()
            .map(|(e, n, v)| format!("{e}:{n}@{v}"))
            .collect()
    }

    #[test]
    fn reads_cargo_lock_registry_packages_only() {
        let text = r#"
version = 4

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "myapp"
version = "0.1.0"

[[package]]
name = "forked"
version = "0.2.0"
source = "git+https://github.com/x/forked#abc"
"#;
        assert_eq!(names(cargo_lock(text)), vec!["crates.io:serde@1.0.200"]);
    }

    #[test]
    fn reads_package_lock_v1_and_v3() {
        let v3 = r#"{"lockfileVersion":3,"packages":{
            "":{"name":"app","version":"1.0.0"},
            "node_modules/lodash":{"version":"4.17.15"},
            "node_modules/a/node_modules/@scope/b":{"version":"2.0.0"},
            "node_modules/local":{"resolved":"../local","link":true}}}"#;
        let mut found = names(package_lock(v3));
        found.sort();
        assert_eq!(found, vec!["npm:@scope/b@2.0.0", "npm:lodash@4.17.15"]);

        let v1 = r#"{"lockfileVersion":1,"dependencies":{
            "a":{"version":"1.0.0","dependencies":{"b":{"version":"2.0.0"}}},
            "c":{"version":"file:../c"}}}"#;
        let mut found = names(package_lock(v1));
        found.sort();
        assert_eq!(found, vec!["npm:a@1.0.0", "npm:b@2.0.0"]);
    }

    #[test]
    fn reads_yarn_lock_classic_and_berry() {
        let classic = "# yarn lockfile v1\n\n\"@babel/core@^7.0.0\", \"@babel/core@^7.1.0\":\n  version \"7.22.5\"\n  dependencies:\n    debug \"^4.1.0\"\n\nlodash@^4.17.0:\n  version \"4.17.15\"\n";
        assert_eq!(
            names(yarn_lock(classic)),
            vec!["npm:@babel/core@7.22.5", "npm:lodash@4.17.15"]
        );
        let berry = "__metadata:\n  version: 6\n\n\"lodash@npm:^4.17.0\":\n  version: 4.17.15\n\n\"app@workspace:.\":\n  version: 0.0.0-use.local\n";
        assert_eq!(names(yarn_lock(berry)), vec!["npm:lodash@4.17.15"]);
    }

    #[test]
    fn reads_pnpm_lock_versions_5_6_and_9() {
        let text = "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    dependencies: {}\n\npackages:\n\n  /old/1.0.0:\n    resolution: {}\n  /@scope/old/1.1.0_react@18.2.0:\n    resolution: {}\n  /mid@2.0.0:\n    resolution: {}\n  new@3.0.0(react@18.2.0):\n    resolution: {}\n  '@scope/new@4.0.0':\n    resolution: {}\n\nsnapshots:\n\n  ignored@9.9.9: {}\n";
        assert_eq!(
            names(pnpm_lock(text)),
            vec![
                "npm:old@1.0.0",
                "npm:@scope/old@1.1.0",
                "npm:mid@2.0.0",
                "npm:new@3.0.0",
                "npm:@scope/new@4.0.0"
            ]
        );
    }

    #[test]
    fn reads_python_lockfiles() {
        let req = "# pinned\nrequests==2.19.0\nDjango[argon2]==3.2.0 ; python_version > '3'\nflask>=2\n-e .\n";
        assert_eq!(
            names(requirements_txt(req)),
            vec!["PyPI:requests@2.19.0", "PyPI:Django@3.2.0"]
        );
        let uv = "[[package]]\nname = \"app\"\nversion = \"0.1.0\"\nsource = { editable = \".\" }\n\n[[package]]\nname = \"jinja2\"\nversion = \"2.10\"\nsource = { registry = \"https://pypi.org/simple\" }\n";
        assert_eq!(names(python_toml_lock(uv)), vec!["PyPI:jinja2@2.10"]);
        let pipfile = r#"{"default":{"urllib3":{"version":"==1.24.1"}},"develop":{"pytest":{"version":"==7.0.0"}}}"#;
        let mut found = names(pipfile_lock(pipfile));
        found.sort();
        assert_eq!(found, vec!["PyPI:pytest@7.0.0", "PyPI:urllib3@1.24.1"]);
    }

    #[test]
    fn reads_go_ruby_and_php_lockfiles() {
        let gomod = "module x\n\ngo 1.21\n\nrequire github.com/a/b v1.2.3\n\nrequire (\n\tgolang.org/x/net v0.17.0 // indirect\n\tgithub.com/c/d v2.0.0+incompatible\n)\n";
        assert_eq!(
            names(go_mod(gomod)),
            vec![
                "Go:github.com/a/b@1.2.3",
                "Go:golang.org/x/net@0.17.0",
                "Go:github.com/c/d@2.0.0+incompatible"
            ]
        );
        let gems = "GEM\n  remote: https://rubygems.org/\n  specs:\n    nokogiri (1.16.0-x86_64-linux)\n      racc (~> 1.4)\n    rack (2.2.3)\n\nPLATFORMS\n  ruby\n";
        assert_eq!(
            names(gemfile_lock(gems)),
            vec!["RubyGems:nokogiri@1.16.0", "RubyGems:rack@2.2.3"]
        );
        let composer = r#"{"packages":[{"name":"guzzlehttp/guzzle","version":"7.4.0"}],"packages-dev":[{"name":"phpunit/phpunit","version":"v9.5.0"},{"name":"x/y","version":"dev-main"}]}"#;
        assert_eq!(
            names(composer_lock(composer)),
            vec![
                "Packagist:guzzlehttp/guzzle@7.4.0",
                "Packagist:phpunit/phpunit@9.5.0"
            ]
        );
    }

    #[test]
    fn collects_lockfiles_but_not_installed_packages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("web/node_modules/x")).unwrap();
        fs::write(root.join("requirements.txt"), "requests==2.19.0\n").unwrap();
        fs::write(
            root.join("web/package-lock.json"),
            r#"{"packages":{"node_modules/lodash":{"version":"4.17.15"}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("web/node_modules/x/package-lock.json"),
            r#"{"packages":{"node_modules/evil":{"version":"1.0.0"}}}"#,
        )
        .unwrap();
        let (packages, lockfiles) = collect(root);
        assert_eq!(lockfiles, vec!["requirements.txt", "web/package-lock.json"]);
        assert_eq!(packages.len(), 2);
        let lodash = packages.iter().find(|p| p.name == "lodash").unwrap();
        assert_eq!(lodash.file, "web/package-lock.json");
    }

    #[test]
    fn scores_cvss_vectors() {
        assert_eq!(
            cvss3_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            Some(9.8)
        );
        assert_eq!(
            cvss3_score("CVSS:3.1/AV:N/AC:L/PR:H/UI:N/S:U/C:H/I:H/A:H"),
            Some(7.2)
        );
        assert_eq!(
            cvss3_score("CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:N/A:N"),
            Some(5.9)
        );
        assert_eq!(
            cvss3_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N"),
            Some(6.1)
        );
        assert_eq!(
            cvss3_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N"),
            Some(0.0)
        );
        assert_eq!(cvss3_score("garbage"), None);
    }

    #[test]
    fn compares_versions_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Greater);
        assert_eq!(compare_versions("4.17.21", "4.17.21"), Equal);
        assert_eq!(compare_versions("2.0", "2.0.1"), Less);
    }

    struct Fake {
        gets: Mutex<usize>,
        fail: bool,
    }

    impl Source for Fake {
        fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
            if self.fail {
                return Err("connection refused".into());
            }
            assert_eq!(path, "/querybatch");
            let results: Vec<Value> = body["queries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|q| match q["package"]["name"].as_str().unwrap() {
                    "lodash" => {
                        json!({"vulns": [{"id": "GHSA-1"}, {"id": "GHSA-2"}, {"id": "GHSA-old"}]})
                    }
                    "derivative" => json!({"vulns": [{"id": "RUSTSEC-1"}]}),
                    _ => json!({}),
                })
                .collect();
            Ok(json!({ "results": results }))
        }

        fn get(&self, path: &str) -> Result<Value, String> {
            *self.gets.lock().unwrap() += 1;
            let affected = |eco: &str, name: &str, fixed: Option<&str>| {
                let mut events = vec![json!({"introduced": "0"})];
                if let Some(f) = fixed {
                    events.push(json!({"fixed": f}));
                }
                json!([{"package": {"ecosystem": eco, "name": name},
                        "ranges": [{"type": "SEMVER", "events": events}]}])
            };
            Ok(match path {
                "/vulns/GHSA-1" => json!({"id": "GHSA-1", "summary": "Prototype pollution",
                    "database_specific": {"severity": "MODERATE"},
                    "affected": affected("npm", "lodash", Some("4.17.19"))}),
                "/vulns/GHSA-2" => json!({"id": "GHSA-2", "summary": "Command injection",
                    "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L/PR:H/UI:N/S:U/C:H/I:H/A:H"}],
                    "affected": affected("npm", "lodash", Some("4.17.21"))}),
                "/vulns/GHSA-old" => json!({"id": "GHSA-old", "withdrawn": "2020-01-01T00:00:00Z",
                    "affected": affected("npm", "lodash", Some("5.0.0"))}),
                "/vulns/RUSTSEC-1" => {
                    json!({"id": "RUSTSEC-1", "summary": "`derivative` is unmaintained",
                    "affected": [{"package": {"ecosystem": "crates.io", "name": "derivative"},
                        "database_specific": {"informational": "unmaintained"},
                        "ranges": [{"type": "SEMVER", "events": [{"introduced": "0.0.0-0"}]}]}]})
                }
                _ => return Err("not found".into()),
            })
        }
    }

    #[test]
    fn turns_advisories_into_findings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package-lock.json"),
            r#"{"packages":{"node_modules/lodash":{"version":"4.17.15"},"node_modules/react":{"version":"18.2.0"}}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("Cargo.lock"),
            "[[package]]\nname = \"derivative\"\nversion = \"2.2.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )
        .unwrap();
        let fake = Fake {
            gets: Mutex::new(0),
            fail: false,
        };
        let result = check(dir.path(), &fake);
        assert_eq!(result.packages, 3);
        assert_eq!(result.lockfiles, vec!["Cargo.lock", "package-lock.json"]);
        assert!(result.error.is_none());
        assert_eq!(*fake.gets.lock().unwrap(), 4);

        let lodash = result
            .findings
            .iter()
            .find(|f| f.file == "package-lock.json")
            .unwrap();
        assert_eq!(lodash.severity, Severity::High);
        assert_eq!(lodash.check, VULNERABLE_DEPENDENCY);
        assert_eq!(
            lodash.message,
            "lodash 4.17.15 (npm) has 2 known vulnerabilities, e.g. \"Command injection\": GHSA-2, GHSA-1. Fixed in 4.17.21."
        );
        let derivative = result
            .findings
            .iter()
            .find(|f| f.file == "Cargo.lock")
            .unwrap();
        assert_eq!(derivative.severity, Severity::Low);
        assert!(derivative.message.ends_with("No fixed version listed."));
    }

    #[test]
    fn reports_an_unreachable_database() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("requirements.txt"), "requests==2.19.0\n").unwrap();
        let result = check(
            dir.path(),
            &Fake {
                gets: Mutex::new(0),
                fail: true,
            },
        );
        assert!(result.findings.is_empty());
        assert_eq!(result.packages, 0);
        assert!(result.error.unwrap().contains("osv.dev"));
    }
}
