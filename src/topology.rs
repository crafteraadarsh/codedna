//! What a project's deployment configuration says will run.
//!
//! Reads Kubernetes manifests (including Helm chart templates and Knative /
//! Cloud Run services), Docker Compose, Serverless Framework, AWS SAM and
//! CloudFormation, Vercel, Netlify, Cloudflare Workers, Fly.io, Render, App
//! Engine and Procfiles. Each workload, function or process becomes a
//! [`Unit`]: what runs, how traffic or events reach it, how it scales, and
//! which folder or file in the repository holds its code.
//!
//! Everything is read, never run or deployed. Values that are computed at
//! deploy time (Helm values, CloudFormation references, environment
//! variables) are shown as written. Files in test, example and docs folders
//! are skipped and counted, since they rarely describe what really runs.

use crate::analysis::dotted;
use crate::safe_text::sanitize;
use crate::yaml::{self, Node, Value};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// Most units returned.
pub const MAX_UNITS: usize = 500;
/// Larger files are skipped: deployment files are small.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// Longest text shown for one value.
const MAX_TEXT: usize = 160;
/// Most triggers listed for one unit.
const MAX_TRIGGERS: usize = 20;

/// Folders whose files rarely describe what really runs.
const EXAMPLE_DIRS: &[&str] = &[
    "test",
    "tests",
    "testdata",
    "test-data",
    "__tests__",
    "fixtures",
    "fixture",
    "e2e",
    "example",
    "examples",
    "sample",
    "samples",
    "docs",
    "doc",
    "vendor",
    "third_party",
    "node_modules",
];

/// Extensions of files that can hold a function's code.
const HANDLER_EXTENSIONS: &[&str] = &["ts", "js", "mjs", "cjs", "mts", "tsx", "jsx", "py", "rb"];

/// One thing the configuration says will run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Unit {
    /// `Kubernetes`, `Helm`, `Knative`, `Docker Compose`, `Serverless
    /// Framework`, `AWS SAM`, `CloudFormation`, `Vercel`, `Netlify`,
    /// `Cloudflare Workers`, `Fly.io`, `Render`, `App Engine` or `Procfile`.
    pub platform: String,
    /// What the platform calls it: `Deployment`, `CronJob`, `service`,
    /// `function`, `worker`, `process`, …
    pub kind: String,
    pub name: String,
    /// File that declares it, relative to the scanned folder.
    pub file: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Container images it runs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Function entry point as written, e.g. `src/users.create`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// Ports it listens on or publishes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replicas: Option<u64>,
    /// Autoscaling range, e.g. `2-10`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoscale: Option<String>,
    /// Cron expression or rate it runs on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// How requests or events reach it: services, ingress hosts and paths,
    /// HTTP routes, queues.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<String>,
    /// Other units it waits for before starting (Compose `depends_on`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Folders or files in the repository that hold its code.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code: Vec<String>,
    /// How `code` was found: `handler`, `build context`, `skaffold`,
    /// `image name`, `entry file`, `root dir` or `config folder`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_from: Option<&'static str>,
    /// Pod labels, used to match Services.
    #[serde(skip)]
    labels: Vec<(String, String)>,
}

/// Everything found.
#[derive(Debug, Clone, Serialize)]
pub struct Topology {
    pub files_scanned: usize,
    /// Deployment files that declared something.
    pub config_files: Vec<String>,
    /// Deployment-looking files skipped because they sit in test, example or
    /// docs folders.
    pub skipped_example_files: usize,
    pub units: Vec<Unit>,
    pub truncated: bool,
}

/// Read every deployment file under `root`.
pub fn topology(root: &Path) -> Topology {
    let files = crate::scanner::scan_repository(root);
    let relative: Vec<String> = files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(root).unwrap_or(path);
            sanitize(&relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let repo = Repo::new(&relative, root);
    let candidates: Vec<(usize, Kind)> = relative
        .iter()
        .enumerate()
        .filter_map(|(i, f)| kind_of(f, &repo).map(|k| (i, k)))
        .collect();
    // Any YAML file could be a manifest, so only count skipped ones that
    // declare a workload.
    let skipped = candidates
        .par_iter()
        .filter(|(i, _)| in_example_dir(&relative[*i]))
        .filter(|&&(i, kind)| kind != Kind::Manifest || declares_workload(&files[i]))
        .count();
    let found: Vec<Found> = candidates
        .par_iter()
        .filter(|(i, _)| !in_example_dir(&relative[*i]))
        .filter_map(|&(i, kind)| {
            let meta = std::fs::metadata(&files[i]).ok()?;
            if meta.len() > MAX_FILE_BYTES {
                return None;
            }
            let bytes = std::fs::read(&files[i]).ok()?;
            Some(read_file(
                &relative[i],
                kind,
                &String::from_utf8_lossy(&bytes),
                &repo,
            ))
        })
        .collect();
    let mut topology = link(files.len(), found, &repo);
    topology.skipped_example_files = skipped;
    topology
}

/// Whether a small YAML file names a workload or function kind.
fn declares_workload(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() > MAX_FILE_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    String::from_utf8_lossy(&bytes).lines().any(|line| {
        let line = line.trim_start();
        let Some(value) = line
            .strip_prefix("kind:")
            .or_else(|| line.strip_prefix("Type:"))
        else {
            return false;
        };
        matches!(
            value.trim().trim_matches(|c| c == '"' || c == '\''),
            "Deployment"
                | "StatefulSet"
                | "DaemonSet"
                | "Job"
                | "CronJob"
                | "Pod"
                | "AWS::Serverless::Function"
                | "AWS::Lambda::Function"
        )
    })
}

// ---------------------------------------------------------------------------
// The repository's files
// ---------------------------------------------------------------------------

struct Repo<'a> {
    files: BTreeSet<&'a str>,
    dirs: BTreeSet<String>,
    /// Folders holding a Dockerfile, by folder name.
    docker_dirs: HashMap<String, Vec<String>>,
    /// Folders holding a Helm `Chart.yaml`.
    charts: BTreeSet<String>,
}

impl<'a> Repo<'a> {
    fn new(relative: &'a [String], root: &Path) -> Self {
        let mut dirs = BTreeSet::new();
        let mut docker_dirs: HashMap<String, Vec<String>> = HashMap::new();
        let mut charts = BTreeSet::new();
        let root_name = root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        for file in relative {
            let mut dir = folder(file);
            let name = file_name(file);
            if name == "Dockerfile" {
                let key = if dir.is_empty() {
                    root_name.clone()
                } else {
                    file_name(dir).to_lowercase()
                };
                docker_dirs.entry(key).or_default().push(dir.to_string());
            }
            if name == "Chart.yaml" {
                charts.insert(dir.to_string());
            }
            while !dir.is_empty() && dirs.insert(dir.to_string()) {
                dir = folder(dir);
            }
        }
        Repo {
            files: relative.iter().map(String::as_str).collect(),
            dirs,
            docker_dirs,
            charts,
        }
    }

    fn is_dir(&self, dir: &str) -> bool {
        dir.is_empty() || self.dirs.contains(dir)
    }

    /// The chart whose `templates/` folder holds `file`.
    fn chart_of<'f>(&self, file: &'f str) -> Option<&'f str> {
        let mut dir = folder(file);
        while !dir.is_empty() {
            if file_name(dir) == "templates" && self.charts.contains(folder(dir)) {
                return Some(folder(dir));
            }
            dir = folder(dir);
        }
        None
    }
}

fn folder(file: &str) -> &str {
    file.rfind('/').map_or("", |i| &file[..i])
}

fn file_name(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

fn extension(name: &str) -> &str {
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

fn in_example_dir(file: &str) -> bool {
    let dir = folder(file);
    !dir.is_empty()
        && dir
            .split('/')
            .any(|s| EXAMPLE_DIRS.contains(&s.to_lowercase().as_str()))
}

/// `dir` joined with a relative path, `.` and `..` resolved. `None` when the
/// path leaves the repository or is absolute.
fn join(dir: &str, path: &str) -> Option<String> {
    let path = path.trim();
    if path.starts_with('/') || path.contains("://") || path.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Display form of a folder.
fn shown_dir(dir: &str) -> String {
    if dir.is_empty() {
        "./".to_string()
    } else {
        format!("{}/", dotted(dir))
    }
}

// ---------------------------------------------------------------------------
// Which files are read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Procfile,
    Serverless,
    Vercel,
    Netlify,
    Wrangler,
    Fly,
    Render,
    AppEngine,
    Skaffold,
    Compose,
    /// Kubernetes, Helm, Knative or CloudFormation.
    Manifest,
}

fn kind_of(file: &str, repo: &Repo) -> Option<Kind> {
    let name = file_name(file);
    let lower = name.to_lowercase();
    let ext = extension(&lower);
    Some(match lower.as_str() {
        "procfile" => Kind::Procfile,
        "serverless.yml" | "serverless.yaml" => Kind::Serverless,
        "vercel.json" => Kind::Vercel,
        "netlify.toml" => Kind::Netlify,
        "wrangler.toml" | "wrangler.json" => Kind::Wrangler,
        "fly.toml" => Kind::Fly,
        "render.yaml" | "render.yml" => Kind::Render,
        "app.yaml" | "app.yml" => Kind::AppEngine,
        "skaffold.yaml" | "skaffold.yml" => Kind::Skaffold,
        _ if is_compose(&lower) => Kind::Compose,
        _ if matches!(ext, "yaml" | "yml") => Kind::Manifest,
        _ if lower.ends_with("template.json") => Kind::Manifest,
        _ if ext == "json" && repo.chart_of(file).is_some() => Kind::Manifest,
        _ => return None,
    })
}

fn is_compose(lower: &str) -> bool {
    let Some(stem) = lower
        .strip_suffix(".yml")
        .or_else(|| lower.strip_suffix(".yaml"))
    else {
        return false;
    };
    stem == "compose"
        || stem == "docker-compose"
        || stem.starts_with("docker-compose.")
        || stem.starts_with("compose.")
}

// ---------------------------------------------------------------------------
// Reading one file
// ---------------------------------------------------------------------------

/// A Kubernetes Service.
struct KService {
    file: String,
    name: String,
    namespace: String,
    selector: Vec<(String, String)>,
    text: String,
}

/// An Ingress or HTTPRoute rule: the Service it sends to, and the text.
struct KRoute {
    file: String,
    namespace: String,
    service: String,
    text: String,
}

/// An autoscaler and the workload it scales.
struct Scaler {
    file: String,
    namespace: String,
    target: String,
    range: String,
    triggers: Vec<String>,
}

#[derive(Default)]
struct Found {
    file: String,
    units: Vec<Unit>,
    services: Vec<KService>,
    routes: Vec<KRoute>,
    scalers: Vec<Scaler>,
    /// Image name → folder that builds it.
    builds: Vec<(String, String)>,
}

fn read_file(file: &str, kind: Kind, text: &str, repo: &Repo) -> Found {
    let mut found = Found {
        file: file.to_string(),
        ..Found::default()
    };
    let dir = folder(file);
    match kind {
        Kind::Procfile => procfile(file, text, &mut found),
        Kind::Netlify | Kind::Wrangler | Kind::Fly if file.ends_with(".toml") => {
            let Some(doc) = toml_doc(text) else {
                return found;
            };
            match kind {
                Kind::Netlify => netlify(file, &doc, text, repo, &mut found),
                Kind::Wrangler => wrangler(file, &doc, text, &mut found),
                _ => fly(file, &doc, text, repo, &mut found),
            }
        }
        _ => {
            let chart = repo.chart_of(file);
            let (source, templates) = match chart {
                Some(_) => helm_template(text),
                None => (text.to_string(), Vec::new()),
            };
            if kind == Kind::Manifest
                && !source.contains("apiVersion")
                && !source.contains("Resources")
            {
                return found;
            }
            for doc in yaml::load(&source) {
                match kind {
                    Kind::Serverless => serverless(file, &doc, repo, &mut found),
                    Kind::Vercel => vercel(file, &doc, repo, &mut found),
                    Kind::Wrangler => wrangler(file, &doc, text, &mut found),
                    Kind::Render => render(file, &doc, repo, &mut found),
                    Kind::Skaffold => skaffold(dir, &doc, &mut found),
                    Kind::Compose => compose(file, &doc, repo, &mut found),
                    Kind::AppEngine if doc.get("runtime").is_some() => {
                        app_engine(file, &doc, &mut found)
                    }
                    _ => {
                        if !kubernetes(file, &doc, chart, &mut found) {
                            cloudformation(file, &doc, repo, &mut found);
                        }
                    }
                }
            }
            if !templates.is_empty() {
                for unit in &mut found.units {
                    restore_unit(unit, &templates);
                }
                for s in &mut found.services {
                    s.name = restore(&s.name, &templates);
                    s.namespace = restore(&s.namespace, &templates);
                    s.text = restore(&s.text, &templates);
                    for (k, v) in &mut s.selector {
                        *k = restore(k, &templates);
                        *v = restore(v, &templates);
                    }
                }
                for r in &mut found.routes {
                    r.namespace = restore(&r.namespace, &templates);
                    r.service = restore(&r.service, &templates);
                    r.text = restore(&r.text, &templates);
                }
                for s in &mut found.scalers {
                    s.namespace = restore(&s.namespace, &templates);
                    s.target = restore(&s.target, &templates);
                }
            }
        }
    }
    for unit in &mut found.units {
        unit.file = file.to_string();
    }
    found
}

/// Sanitize, shorten, and hide text that looks like a credential.
fn clean(s: &str) -> String {
    if !crate::audit::find_secrets(s).is_empty() {
        return "(hidden: looks like it contains a credential)".to_string();
    }
    let s = sanitize(s.trim());
    if s.chars().count() <= MAX_TEXT {
        return s;
    }
    let mut short: String = s.chars().take(MAX_TEXT).collect();
    short.push('…');
    short
}

fn opt(node: Option<&Node>) -> Option<String> {
    node.and_then(Node::words)
        .filter(|s| !s.trim().is_empty())
        .map(|s| clean(&s))
}

fn number(node: Option<&Node>) -> Option<u64> {
    node.and_then(Node::str).and_then(|s| s.trim().parse().ok())
}

/// The 1-based line of the first line containing `needle`, else 1.
fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|l| l.contains(needle))
        .map_or(1, |i| i + 1)
}

fn unit(platform: &str, kind: &str, name: &str, line: usize) -> Unit {
    Unit {
        platform: platform.to_string(),
        kind: kind.to_string(),
        name: clean(name),
        line,
        ..Unit::default()
    }
}

/// The last segment of an ARN, which names the queue, topic or bucket
/// without the account number.
fn short_arn(s: &str) -> &str {
    if s.starts_with("arn:") {
        s.rsplit([':', '/']).next().unwrap_or(s)
    } else {
        s
    }
}

// ── Helm ─────────────────────────────────────────────────────────────────────

/// Make a Helm template parse as YAML: drop lines that are only template
/// actions, and replace inline `{{ … }}` with plain tokens restored later.
fn helm_template(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut templates = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("{{") && trimmed.ends_with("}}") && !trimmed[2..].contains("{{") {
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find("{{") {
            let Some(len) = rest[start..].find("}}") else {
                break;
            };
            out.push_str(&rest[..start]);
            // The same action gets the same token, so a selector and the
            // labels it picks still match.
            let action = &rest[start..start + len + 2];
            let key = |a: &str| -> String {
                a.chars()
                    .filter(|c| !c.is_whitespace() && *c != '-')
                    .collect()
            };
            let index = match templates
                .iter()
                .position(|t: &String| key(t) == key(action))
            {
                Some(i) => i,
                None => {
                    templates.push(action.to_string());
                    templates.len() - 1
                }
            };
            out.push_str(&format!("HELMTPL{index}X"));
            rest = &rest[start + len + 2..];
        }
        out.push_str(rest);
        out.push('\n');
    }
    (out, templates)
}

fn restore(s: &str, templates: &[String]) -> String {
    if !s.contains("HELMTPL") {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("HELMTPL") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 7..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        match (after[..digits].parse::<usize>().ok(), &after[digits..]) {
            (Some(i), tail) if tail.starts_with('X') && i < templates.len() => {
                out.push_str(&templates[i]);
                rest = &tail[1..];
            }
            _ => {
                out.push_str("HELMTPL");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    clean(&out)
}

fn restore_unit(u: &mut Unit, t: &[String]) {
    u.name = restore(&u.name, t);
    if let Some(ns) = &u.namespace {
        u.namespace = Some(restore(ns, t));
    }
    for s in u
        .images
        .iter_mut()
        .chain(&mut u.ports)
        .chain(&mut u.triggers)
        .chain(&mut u.depends_on)
    {
        *s = restore(s, t);
    }
    for s in [&mut u.command, &mut u.schedule].into_iter().flatten() {
        *s = restore(s, t);
    }
    for (k, v) in &mut u.labels {
        *k = restore(k, t);
        *v = restore(v, t);
    }
}

// ── Kubernetes ───────────────────────────────────────────────────────────────

/// Read one Kubernetes object. `false` when `doc` is not one.
fn kubernetes(file: &str, doc: &Node, chart: Option<&str>, found: &mut Found) -> bool {
    let (Some(kind), Some(api)) = (doc.str_at("kind"), doc.str_at("apiVersion")) else {
        return false;
    };
    let name = doc
        .at(&["metadata", "name"])
        .and_then(Node::str)
        .unwrap_or("(unnamed)");
    let namespace = doc
        .at(&["metadata", "namespace"])
        .and_then(Node::str)
        .map(clean);
    let ns_key = namespace_key(namespace.as_deref());
    let platform = match chart {
        Some(c) => format!("Helm chart {}", shown_dir(c)),
        None => "Kubernetes".to_string(),
    };
    let spec = doc.get("spec");
    let at = |keys: &[&str]| spec.and_then(|s| s.at(keys));
    match kind {
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job" | "Rollout" => {
            workload(&platform, kind, name, doc, at(&["template"]), found);
        }
        "CronJob" => {
            if let Some(u) = workload(
                &platform,
                kind,
                name,
                doc,
                at(&["jobTemplate", "spec", "template"]),
                found,
            ) {
                u.schedule = opt(at(&["schedule"]));
            }
        }
        "Pod" => {
            workload(&platform, kind, name, doc, Some(doc), found);
        }
        "Service" if api.starts_with("serving.knative.dev") => {
            let platform = if chart.is_some() {
                platform
            } else {
                "Knative".to_string()
            };
            if let Some(u) = workload(&platform, "Service", name, doc, at(&["template"]), found) {
                u.triggers
                    .push("HTTP (Knative / Cloud Run service)".to_string());
            }
        }
        "Service" => {
            let selector = pairs(at(&["selector"]));
            let ty = at(&["type"])
                .and_then(Node::str)
                .unwrap_or("ClusterIP")
                .to_string();
            let ports: Vec<String> = at(&["ports"])
                .map(Node::items)
                .unwrap_or_default()
                .iter()
                .filter_map(|p| {
                    let port = p.str_at("port")?;
                    Some(match p.str_at("targetPort") {
                        Some(t) if t != port => format!("{port}→{t}"),
                        _ => port.to_string(),
                    })
                })
                .collect();
            let mut text = format!("Service {name} ({ty}");
            if !ports.is_empty() {
                text.push_str(&format!(", port {}", ports.join(", ")));
            }
            text.push(')');
            found.services.push(KService {
                file: file.to_string(),
                name: name.to_string(),
                namespace: ns_key,
                selector,
                text: clean(&text),
            });
        }
        "Ingress" => {
            let mut add = |host: &str, path: &str, backend: Option<&Node>| {
                let service = backend.and_then(|b| {
                    b.at(&["service", "name"])
                        .or_else(|| b.get("serviceName"))
                        .and_then(Node::str)
                });
                if let Some(service) = service {
                    found.routes.push(KRoute {
                        file: file.to_string(),
                        namespace: ns_key.clone(),
                        service: service.to_string(),
                        text: clean(&format!("Ingress {name}: {host}{path}")),
                    });
                }
            };
            if let Some(b) = at(&["defaultBackend"]).or_else(|| at(&["backend"])) {
                add("*", "/", Some(b));
            }
            for rule in at(&["rules"]).map(Node::items).unwrap_or_default() {
                let host = rule.str_at("host").unwrap_or("*");
                for p in rule
                    .at(&["http", "paths"])
                    .map(Node::items)
                    .unwrap_or_default()
                {
                    add(host, p.str_at("path").unwrap_or("/"), p.get("backend"));
                }
            }
        }
        "HTTPRoute" | "GRPCRoute" => {
            let hosts: Vec<&str> = at(&["hostnames"])
                .map(Node::items)
                .unwrap_or_default()
                .iter()
                .filter_map(Node::str)
                .collect();
            let hosts = if hosts.is_empty() { vec!["*"] } else { hosts };
            for rule in at(&["rules"]).map(Node::items).unwrap_or_default() {
                let mut paths: Vec<&str> = rule
                    .get("matches")
                    .map(Node::items)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|m| m.at(&["path", "value"]).and_then(Node::str))
                    .collect();
                if paths.is_empty() {
                    paths.push("/");
                }
                for backend in rule.get("backendRefs").map(Node::items).unwrap_or_default() {
                    let Some(service) = backend.str_at("name") else {
                        continue;
                    };
                    for host in &hosts {
                        for path in &paths {
                            found.routes.push(KRoute {
                                file: file.to_string(),
                                namespace: ns_key.clone(),
                                service: service.to_string(),
                                text: clean(&format!("{kind} {name}: {host}{path}")),
                            });
                        }
                    }
                }
            }
        }
        "HorizontalPodAutoscaler" => {
            if let Some(target) = at(&["scaleTargetRef", "name"]).and_then(Node::str) {
                let min = number(at(&["minReplicas"])).unwrap_or(1);
                let max = at(&["maxReplicas"]).and_then(Node::str).unwrap_or("?");
                found.scalers.push(Scaler {
                    file: file.to_string(),
                    namespace: ns_key,
                    target: target.to_string(),
                    range: clean(&format!("{min}-{max}")),
                    triggers: Vec::new(),
                });
            }
        }
        "ScaledObject" => {
            if let Some(target) = at(&["scaleTargetRef", "name"]).and_then(Node::str) {
                let min = number(at(&["minReplicaCount"])).unwrap_or(0);
                let max = number(at(&["maxReplicaCount"])).unwrap_or(100);
                let triggers = at(&["triggers"])
                    .map(Node::items)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|t| t.str_at("type"))
                    .map(|t| clean(&format!("KEDA {t}")))
                    .collect();
                found.scalers.push(Scaler {
                    file: file.to_string(),
                    namespace: ns_key,
                    target: target.to_string(),
                    range: format!("{min}-{max}"),
                    triggers,
                });
            }
        }
        _ => return false,
    }
    if let Some(u) = found.units.last_mut() {
        if u.namespace.is_none() && u.name == clean(name) {
            u.namespace = namespace;
        }
    }
    true
}

/// Namespaces compare equal when both are unset or `default`.
fn namespace_key(ns: Option<&str>) -> String {
    match ns {
        None | Some("default") => String::new(),
        Some(ns) => ns.to_string(),
    }
}

fn pairs(node: Option<&Node>) -> Vec<(String, String)> {
    node.map(Node::entries)
        .unwrap_or_default()
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.str()?.to_string())))
        .collect()
}

/// Add a workload from its pod template. `None` when it has no containers,
/// as in a Kustomize patch.
fn workload<'f>(
    platform: &str,
    kind: &str,
    name: &str,
    doc: &Node,
    template: Option<&Node>,
    found: &'f mut Found,
) -> Option<&'f mut Unit> {
    let template = template?;
    let containers = template.at(&["spec", "containers"])?.items();
    if containers.is_empty() {
        return None;
    }
    let mut u = unit(platform, kind, name, doc.line);
    u.images = containers
        .iter()
        .filter_map(|c| c.str_at("image"))
        .map(clean)
        .collect();
    let first = &containers[0];
    let command: Vec<String> = [first.get("command"), first.get("args")]
        .into_iter()
        .flatten()
        .filter_map(Node::words)
        .collect();
    if !command.is_empty() {
        u.command = Some(clean(&command.join(" ")));
    }
    u.ports = containers
        .iter()
        .flat_map(|c| c.get("ports").map(Node::items).unwrap_or_default())
        .filter_map(|p| p.str_at("containerPort"))
        .map(clean)
        .collect();
    u.replicas = number(doc.at(&["spec", "replicas"]));
    u.labels = pairs(template.at(&["metadata", "labels"]));
    found.units.push(u);
    found.units.last_mut()
}

// ── Docker Compose and Skaffold ──────────────────────────────────────────────

fn compose(file: &str, doc: &Node, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    for (name, svc) in doc.get("services").map(Node::entries).unwrap_or_default() {
        let mut u = unit("Docker Compose", "service", name, svc.line);
        let image = svc.str_at("image");
        u.images.extend(image.map(clean));
        u.command = opt(svc.get("command")).or_else(|| opt(svc.get("entrypoint")));
        u.ports = svc
            .get("ports")
            .map(Node::items)
            .unwrap_or_default()
            .iter()
            .filter_map(|p| match p.str() {
                Some(s) => Some(clean(s)),
                None => {
                    let target = p.str_at("target")?;
                    Some(clean(&match p.str_at("published") {
                        Some(pub_) => format!("{pub_}:{target}"),
                        None => target.to_string(),
                    }))
                }
            })
            .collect();
        u.replicas = number(svc.at(&["deploy", "replicas"]));
        // A list of names, or a map from name to start condition.
        u.depends_on = match svc.get("depends_on") {
            Some(d) if !d.items().is_empty() => {
                d.items().iter().filter_map(Node::str).map(clean).collect()
            }
            Some(d) => d.entries().iter().map(|(k, _)| clean(k)).collect(),
            None => Vec::new(),
        };
        u.depends_on.truncate(MAX_TRIGGERS);
        let context = match svc.get("build") {
            Some(b) if b.str().is_some() => b.str(),
            Some(b) => Some(b.str_at("context").unwrap_or(".")),
            None => None,
        };
        if let Some(dir) = context
            .and_then(|c| join(dir, c))
            .filter(|d| repo.is_dir(d))
        {
            u.code.push(shown_dir(&dir));
            u.code_from = Some("build context");
            if let Some(image) = image {
                found.builds.push((image_repo(image), dir));
            }
        }
        found.units.push(u);
    }
}

fn skaffold(dir: &str, doc: &Node, found: &mut Found) {
    for artifact in doc
        .at(&["build", "artifacts"])
        .map(Node::items)
        .unwrap_or_default()
    {
        let Some(image) = artifact.str_at("image") else {
            continue;
        };
        let context = artifact.str_at("context").unwrap_or(".");
        if let Some(ctx) = join(dir, context) {
            found.builds.push((image_repo(image), ctx));
        }
    }
}

/// An image name without registry tag or digest, lowercased.
fn image_repo(image: &str) -> String {
    let image = image.split('@').next().unwrap_or(image);
    let last_slash = image.rfind('/').map_or(0, |i| i + 1);
    let end = image[last_slash..]
        .find(':')
        .map_or(image.len(), |i| last_slash + i);
    image[..end].to_lowercase()
}

/// The last path segment of an image name.
fn image_base(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

// ── Serverless Framework ─────────────────────────────────────────────────────

fn serverless(file: &str, doc: &Node, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    let provider_runtime = doc.at(&["provider", "runtime"]).and_then(Node::str);
    let mut functions = doc
        .get("functions")
        .map(Node::entries)
        .unwrap_or_default()
        .to_vec();
    // `functions` may also be a list of single-entry mappings.
    for item in doc.get("functions").map(Node::items).unwrap_or_default() {
        functions.extend(item.entries().iter().cloned());
    }
    for (name, f) in &functions {
        let mut u = unit("Serverless Framework", "function", name, f.line);
        u.runtime = f.str_at("runtime").or(provider_runtime).map(clean);
        let handler = f.str_at("handler");
        u.handler = handler.map(clean);
        u.images.extend(
            f.get("image")
                .and_then(|i| i.str().or_else(|| i.str_at("name")))
                .map(clean),
        );
        for event in f.get("events").map(Node::items).unwrap_or_default() {
            for (key, value) in event.entries() {
                serverless_event(key, value, &mut u);
            }
        }
        if let Some(code) = handler.and_then(|h| resolve_handler(dir, h, repo)) {
            u.code.push(dotted(&code));
            u.code_from = Some("handler");
        }
        found.units.push(u);
    }
}

fn serverless_event(key: &str, value: &Node, u: &mut Unit) {
    let http = |value: &Node| -> String {
        match value.str() {
            Some("*") => "HTTP any route".to_string(),
            Some(s) => format!("HTTP {s}"),
            None => {
                let method = value.str_at("method").unwrap_or("ANY").to_uppercase();
                let path = value.str_at("path").unwrap_or("/");
                let path = if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/{path}")
                };
                format!("HTTP {method} {path}")
            }
        }
    };
    let text = match key {
        "http" | "httpApi" => http(value),
        "schedule" => {
            let rate = value.str().map(str::to_string).or_else(|| {
                let rate = value.get("rate")?;
                rate.str()
                    .map(str::to_string)
                    .or_else(|| rate.items().first()?.str().map(str::to_string))
            });
            if let Some(rate) = rate {
                u.schedule = Some(clean(&rate));
            }
            return;
        }
        "websocket" => format!(
            "WebSocket {}",
            value
                .str()
                .or_else(|| value.str_at("route"))
                .unwrap_or("route")
        ),
        other => {
            let label = match other {
                "sqs" => "SQS",
                "sns" => "SNS",
                "s3" => "S3",
                "stream" => "stream",
                "eventBridge" => "EventBridge",
                "alb" => "ALB",
                "kafka" => "Kafka",
                "cloudFront" => "CloudFront",
                o => o,
            };
            let detail = value
                .str()
                .or_else(|| value.str_at("arn"))
                .or_else(|| value.str_at("bucket"))
                .or_else(|| value.str_at("topicName"))
                .map(short_arn);
            match detail {
                Some(d) => format!("{label} {d}"),
                None => label.to_string(),
            }
        }
    };
    if u.triggers.len() < MAX_TRIGGERS {
        u.triggers.push(clean(&text));
    }
}

/// The file a handler such as `src/users.create` or `app.lambda_handler`
/// points to, relative to `dir`.
fn resolve_handler(dir: &str, handler: &str, repo: &Repo) -> Option<String> {
    if handler.contains("::") {
        return None;
    }
    let module = &handler[..handler.rfind('.')?];
    let dotted_module = module.replace('.', "/");
    for module in [module, dotted_module.as_str()] {
        let base = join(dir, module)?;
        for ext in HANDLER_EXTENSIONS {
            let candidate = format!("{base}.{ext}");
            if repo.files.contains(candidate.as_str()) {
                return Some(candidate);
            }
        }
    }
    None
}

// ── AWS SAM and CloudFormation ───────────────────────────────────────────────

fn cloudformation(file: &str, doc: &Node, repo: &Repo, found: &mut Found) {
    let Some(resources) = doc.get("Resources") else {
        return;
    };
    let sam = doc
        .get("Transform")
        .and_then(Node::words)
        .is_some_and(|t| t.contains("AWS::Serverless"));
    let typed = |r: &Node| r.str_at("Type").is_some_and(|t| t.starts_with("AWS::"));
    if !sam
        && doc.get("AWSTemplateFormatVersion").is_none()
        && !resources.entries().iter().any(|(_, r)| typed(r))
    {
        return;
    }
    let platform = if sam { "AWS SAM" } else { "CloudFormation" };
    let dir = folder(file);
    let globals = doc.at(&["Globals", "Function"]);
    let global = |key: &str| globals.and_then(|g| g.str_at(key));
    let empty = Node {
        line: 0,
        value: Value::Null,
    };
    for (name, r) in resources.entries() {
        let props = r.get("Properties").unwrap_or(&empty);
        match r.str_at("Type").unwrap_or("") {
            "AWS::Serverless::Function" | "AWS::Lambda::Function" => {
                let mut u = unit(platform, "function", name, r.line);
                let handler = props.str_at("Handler").or_else(|| global("Handler"));
                u.handler = handler.map(clean);
                u.runtime = props
                    .str_at("Runtime")
                    .or_else(|| global("Runtime"))
                    .map(clean);
                u.images.extend(props.str_at("ImageUri").map(clean));
                for (_, event) in props.get("Events").map(Node::entries).unwrap_or_default() {
                    sam_event(event, &mut u);
                }
                let code_dir = props
                    .str_at("CodeUri")
                    .or_else(|| global("CodeUri"))
                    .or_else(|| r.at(&["Metadata", "DockerContext"]).and_then(Node::str))
                    .and_then(|c| join(dir, c))
                    .filter(|d| repo.is_dir(d));
                let resolved = match (&code_dir, handler) {
                    (Some(d), Some(h)) => resolve_handler(d, h, repo),
                    (None, Some(h)) if props.get("CodeUri").is_none() => {
                        resolve_handler(dir, h, repo)
                    }
                    _ => None,
                };
                if let Some(file) = resolved {
                    u.code.push(dotted(&file));
                    u.code_from = Some("handler");
                } else if let Some(d) = code_dir.filter(|d| !d.is_empty()) {
                    u.code.push(shown_dir(&d));
                    u.code_from = Some("handler");
                }
                found.units.push(u);
            }
            "AWS::ECS::TaskDefinition" => {
                let containers = props
                    .get("ContainerDefinitions")
                    .map(Node::items)
                    .unwrap_or_default();
                let mut u = unit(platform, "ECS task", name, r.line);
                u.images = containers
                    .iter()
                    .filter_map(|c| c.str_at("Image"))
                    .map(clean)
                    .collect();
                u.ports = containers
                    .iter()
                    .flat_map(|c| c.get("PortMappings").map(Node::items).unwrap_or_default())
                    .filter_map(|p| p.str_at("ContainerPort"))
                    .map(clean)
                    .collect();
                if !u.images.is_empty() {
                    found.units.push(u);
                }
            }
            _ => {}
        }
    }
}

fn sam_event(event: &Node, u: &mut Unit) {
    let ty = event.str_at("Type").unwrap_or("");
    let p = event.get("Properties");
    let get = |k: &str| p.and_then(|p| p.str_at(k));
    let text = match ty {
        "Api" | "HttpApi" => match get("Path") {
            Some(path) => format!(
                "HTTP {} {path}",
                get("Method").unwrap_or("ANY").to_uppercase()
            ),
            None => "HTTP any route".to_string(),
        },
        "Schedule" | "ScheduleV2" => {
            if let Some(s) = get("Schedule").or_else(|| get("ScheduleExpression")) {
                u.schedule = Some(clean(s));
            }
            return;
        }
        "" => return,
        other => other.to_string(),
    };
    if u.triggers.len() < MAX_TRIGGERS {
        u.triggers.push(clean(&text));
    }
}

// ── Vercel, Netlify, Cloudflare, Fly.io, Render, App Engine, Procfile ────────

fn vercel(file: &str, doc: &Node, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    let api = join(dir, "api").unwrap_or_default();
    let prefix = format!("{api}/");
    for f in repo.files.range(prefix.as_str()..) {
        if !f.starts_with(&prefix) {
            break;
        }
        let rest = &f[prefix.len()..];
        let name = file_name(rest);
        let ext = extension(name);
        let hidden = rest.split('/').any(|s| s.starts_with(['_', '.']));
        let test = name.contains(".test.") || name.contains(".spec.") || name.ends_with(".d.ts");
        if hidden || test || !HANDLER_EXTENSIONS.contains(&ext) && ext != "go" {
            continue;
        }
        let stem = &rest[..rest.len() - ext.len() - 1];
        let route = stem
            .strip_suffix("/index")
            .or(if stem == "index" { Some("") } else { None });
        let route = format!("/api/{}", route.unwrap_or(stem))
            .trim_end_matches('/')
            .to_string();
        let mut u = unit("Vercel", "function", &route, 1);
        u.triggers.push(clean(&format!("HTTP {route}")));
        u.code.push(dotted(f));
        u.code_from = Some("entry file");
        u.file = f.to_string();
        found.units.push(u);
    }
    // Function units name their own file; set it before `read_file` does.
    let functions = found.units.len();
    for cron in doc.get("crons").map(Node::items).unwrap_or_default() {
        let (Some(path), Some(schedule)) = (cron.str_at("path"), cron.str_at("schedule")) else {
            continue;
        };
        let route = path.split('?').next().unwrap_or(path);
        if let Some(u) = found.units[..functions]
            .iter_mut()
            .find(|u| u.name == route)
        {
            u.schedule = Some(clean(schedule));
            continue;
        }
        let mut u = unit("Vercel", "cron", path, cron.line);
        u.schedule = Some(clean(schedule));
        u.triggers.push(clean(&format!("calls {path}")));
        found.units.push(u);
    }
    let _ = file;
}

/// Parse TOML into the same tree as YAML.
fn toml_doc(text: &str) -> Option<Node> {
    let table: toml::Table = text.parse().ok()?;
    Some(toml_node(toml::Value::Table(table), 0))
}

fn toml_node(value: toml::Value, depth: usize) -> Node {
    let value = match value {
        _ if depth > 32 => Value::Null,
        toml::Value::String(s) => Value::Str(s),
        toml::Value::Integer(i) => Value::Str(i.to_string()),
        toml::Value::Float(f) => Value::Str(f.to_string()),
        toml::Value::Boolean(b) => Value::Str(b.to_string()),
        toml::Value::Datetime(d) => Value::Str(d.to_string()),
        toml::Value::Array(items) => {
            Value::Seq(items.into_iter().map(|v| toml_node(v, depth + 1)).collect())
        }
        toml::Value::Table(t) => Value::Map(
            t.into_iter()
                .map(|(k, v)| (k, toml_node(v, depth + 1)))
                .collect(),
        ),
    };
    Node { line: 1, value }
}

fn netlify(file: &str, doc: &Node, text: &str, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    let functions_dir = doc
        .at(&["functions", "directory"])
        .or_else(|| doc.at(&["build", "functions"]))
        .and_then(Node::str)
        .unwrap_or("netlify/functions");
    let Some(fdir) = join(dir, functions_dir) else {
        return;
    };
    let prefix = format!("{fdir}/");
    let mut seen = BTreeSet::new();
    for f in repo.files.range(prefix.as_str()..) {
        if !f.starts_with(&prefix) {
            break;
        }
        let rest = &f[prefix.len()..];
        let parts: Vec<&str> = rest.split('/').collect();
        let ext = extension(parts[parts.len() - 1]);
        if !HANDLER_EXTENSIONS.contains(&ext) && ext != "go" {
            continue;
        }
        let stem = |s: &'_ str| s[..s.len() - ext.len() - 1].to_string();
        let name = match parts.as_slice() {
            [one] => stem(one),
            [d, file] if stem(file) == "index" || stem(file) == *d => d.to_string(),
            _ => continue,
        };
        if name.starts_with(['_', '.']) || !seen.insert(name.clone()) {
            continue;
        }
        let mut u = unit("Netlify", "function", &name, 1);
        u.triggers
            .push(clean(&format!("HTTP /.netlify/functions/{name}")));
        u.schedule = opt(doc.at(&["functions", &name, "schedule"]));
        u.code.push(dotted(f));
        u.code_from = Some("entry file");
        u.file = f.to_string();
        found.units.push(u);
    }
    let _ = text;
}

fn wrangler(file: &str, doc: &Node, text: &str, found: &mut Found) {
    let Some(name) = doc.str_at("name") else {
        return;
    };
    let mut u = unit("Cloudflare Workers", "Worker", name, line_of(text, "name"));
    if let Some(main) = doc.str_at("main").and_then(|m| join(folder(file), m)) {
        u.code.push(dotted(&main));
        u.code_from = Some("entry file");
    }
    let crons: Vec<String> = doc
        .at(&["triggers", "crons"])
        .map(Node::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Node::str)
        .map(clean)
        .collect();
    if !crons.is_empty() {
        u.schedule = Some(crons.join(", "));
    }
    let routes = doc.get("routes").map(Node::items).unwrap_or_default();
    for route in routes.iter().chain(doc.get("route")) {
        if let Some(pattern) = route.str().or_else(|| route.str_at("pattern")) {
            u.triggers.push(clean(&format!("route {pattern}")));
        }
    }
    for consumer in doc
        .at(&["queues", "consumers"])
        .map(Node::items)
        .unwrap_or_default()
    {
        if let Some(queue) = consumer.str_at("queue") {
            u.triggers.push(clean(&format!("queue {queue}")));
        }
    }
    u.triggers.truncate(MAX_TRIGGERS);
    found.units.push(u);
}

fn fly(file: &str, doc: &Node, text: &str, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    let app = doc.str_at("app").unwrap_or("(app)");
    let http = doc.get("http_service");
    let http_port = http.and_then(|h| h.str_at("internal_port"));
    let http_processes: Vec<&str> = http
        .and_then(|h| h.get("processes"))
        .map(Node::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Node::str)
        .collect();
    let image = doc.at(&["build", "image"]).and_then(Node::str);
    let dockerfile = doc
        .at(&["build", "dockerfile"])
        .and_then(Node::str)
        .unwrap_or("Dockerfile");
    let code = join(dir, dockerfile)
        .filter(|d| repo.files.contains(d.as_str()))
        .map(|d| shown_dir(folder(&d)));
    let processes = doc.get("processes").map(Node::entries).unwrap_or_default();
    let mut add = |name: String, process: Option<&str>, command: Option<String>| {
        let mut u = unit("Fly.io", "process", &name, line_of(text, "app"));
        u.command = command;
        u.images.extend(image.map(clean));
        let serves =
            process.is_none_or(|p| http_processes.is_empty() || http_processes.contains(&p));
        if let (Some(port), true) = (http_port, serves) {
            u.triggers.push(clean(&format!("HTTP → port {port}")));
            u.ports.push(clean(port));
        }
        if let Some(code) = &code {
            u.code.push(code.clone());
            u.code_from = Some("build context");
        }
        found.units.push(u);
    };
    if processes.is_empty() {
        add(app.to_string(), None, None);
    }
    for (process, cmd) in processes {
        add(format!("{app} {process}"), Some(process), opt(Some(cmd)));
    }
}

fn render(file: &str, doc: &Node, repo: &Repo, found: &mut Found) {
    let dir = folder(file);
    for s in doc.get("services").map(Node::items).unwrap_or_default() {
        let Some(name) = s.str_at("name") else {
            continue;
        };
        let kind = s.str_at("type").unwrap_or("service");
        let mut u = unit("Render", kind, name, s.line);
        u.runtime = s.str_at("runtime").or_else(|| s.str_at("env")).map(clean);
        u.command = opt(s.get("startCommand"));
        u.schedule = opt(s.get("schedule"));
        u.replicas = number(s.get("numInstances"));
        u.images
            .extend(s.at(&["image", "url"]).and_then(Node::str).map(clean));
        if kind == "web" {
            u.triggers.push("HTTP (public web service)".to_string());
        }
        let root = s
            .str_at("rootDir")
            .or_else(|| s.str_at("dockerContext"))
            .and_then(|r| join(dir, r))
            .filter(|d| !d.is_empty() && repo.is_dir(d));
        if let Some(root) = root {
            u.code.push(shown_dir(&root));
            u.code_from = Some("root dir");
        }
        found.units.push(u);
    }
}

fn app_engine(file: &str, doc: &Node, found: &mut Found) {
    let name = doc.str_at("service").unwrap_or("default");
    let mut u = unit("App Engine", "service", name, doc.line);
    u.runtime = opt(doc.get("runtime"));
    u.command = opt(doc.get("entrypoint"));
    for h in doc.get("handlers").map(Node::items).unwrap_or_default() {
        if let Some(url) = h.str_at("url") {
            if u.triggers.len() < MAX_TRIGGERS {
                u.triggers.push(clean(&format!("HTTP {url}")));
            }
        }
    }
    if u.triggers.is_empty() {
        u.triggers.push("HTTP (App Engine service)".to_string());
    }
    u.code.push(shown_dir(folder(file)));
    u.code_from = Some("config folder");
    found.units.push(u);
}

fn procfile(_file: &str, text: &str, found: &mut Found) {
    for (i, line) in text.lines().enumerate() {
        let Some((name, command)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let valid = !name.is_empty()
            && !name.starts_with('#')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid || command.trim().is_empty() {
            continue;
        }
        let mut u = unit("Procfile", "process", name, i + 1);
        u.command = Some(clean(command));
        if name == "web" {
            u.triggers.push("HTTP (web process)".to_string());
        }
        found.units.push(u);
    }
}

// ---------------------------------------------------------------------------
// Linking
// ---------------------------------------------------------------------------

fn is_kubernetes(u: &Unit) -> bool {
    u.platform == "Kubernetes" || u.platform == "Knative" || u.platform.starts_with("Helm chart")
}

/// The items declared in `file`, else in its folder, else all of them.
fn nearest<'a, T>(items: Vec<T>, file: &str, file_of: impl Fn(&T) -> &'a str) -> Vec<T> {
    if items.iter().any(|t| file_of(t) == file) {
        return items.into_iter().filter(|t| file_of(t) == file).collect();
    }
    let dir = folder(file);
    if items.iter().any(|t| folder(file_of(t)) == dir) {
        return items
            .into_iter()
            .filter(|t| folder(file_of(t)) == dir)
            .collect();
    }
    items
}

fn add_trigger(u: &mut Unit, text: &str) {
    if u.triggers.len() < MAX_TRIGGERS && !u.triggers.iter().any(|t| t == text) {
        u.triggers.push(text.to_string());
    }
}

fn link(files_scanned: usize, found: Vec<Found>, repo: &Repo) -> Topology {
    let mut units = Vec::new();
    let mut services = Vec::new();
    let mut routes = Vec::new();
    let mut scalers = Vec::new();
    let mut builds: BTreeMap<String, String> = BTreeMap::new();
    let mut config_files = BTreeSet::new();
    for f in found {
        if !f.units.is_empty() || !f.services.is_empty() || !f.builds.is_empty() {
            config_files.insert(dotted(&f.file));
        }
        units.extend(f.units);
        services.extend(f.services);
        routes.extend(f.routes);
        scalers.extend(f.scalers);
        for (image, dir) in f.builds {
            builds.entry(image).or_insert(dir);
        }
    }

    // Services pick pods by label; with no selector (common in Helm
    // templates, whose labels come from helpers) fall back to the name.
    // Repositories often hold several copies of the same manifests (plain,
    // Kustomize, a release bundle), so each object links within its own
    // file first, then its own folder, then anywhere.
    let service_units: Vec<Vec<usize>> = services
        .iter()
        .map(|s| {
            let matched: Vec<usize> = units
                .iter()
                .enumerate()
                .filter(|(_, u)| is_kubernetes(u))
                .filter(|(_, u)| namespace_key(u.namespace.as_deref()) == s.namespace)
                .filter(|(_, u)| {
                    if s.selector.is_empty() {
                        u.name == clean(&s.name)
                    } else {
                        s.selector.iter().all(|p| u.labels.contains(p))
                    }
                })
                .map(|(i, _)| i)
                .collect();
            nearest(matched, &s.file, |&i| units[i].file.as_str())
        })
        .collect();
    for (s, matched) in services.iter().zip(&service_units) {
        for &i in matched {
            add_trigger(&mut units[i], &s.text);
        }
    }
    for r in &routes {
        let targets: Vec<usize> = (0..services.len())
            .filter(|&i| services[i].namespace == r.namespace && services[i].name == r.service)
            .collect();
        for s in nearest(targets, &r.file, |&i| services[i].file.as_str()) {
            for &i in &service_units[s] {
                add_trigger(&mut units[i], &r.text);
            }
        }
    }
    for s in &scalers {
        let targets: Vec<usize> = (0..units.len())
            .filter(|&i| {
                is_kubernetes(&units[i])
                    && units[i].name == clean(&s.target)
                    && namespace_key(units[i].namespace.as_deref()) == s.namespace
            })
            .collect();
        for i in nearest(targets, &s.file, |&i| units[i].file.as_str()) {
            units[i].autoscale = Some(s.range.clone());
            for t in &s.triggers {
                add_trigger(&mut units[i], t);
            }
        }
    }

    // Find the code behind container images.
    for u in units.iter_mut().filter(|u| u.code.is_empty()) {
        for image in &u.images {
            let repo_name = image_repo(image);
            if let Some(dir) = builds.get(&repo_name) {
                u.code.push(shown_dir(dir));
                u.code_from = Some(if u.platform == "Docker Compose" {
                    "build context"
                } else {
                    "skaffold"
                });
                continue;
            }
            if let Some(dir) = builds
                .iter()
                .find(|(k, _)| image_base(k) == image_base(&repo_name))
                .map(|(_, d)| d)
            {
                u.code.push(shown_dir(dir));
                u.code_from = Some("image name");
                continue;
            }
            if let Some([dir]) = repo
                .docker_dirs
                .get(image_base(&repo_name))
                .map(Vec::as_slice)
            {
                u.code.push(shown_dir(dir));
                u.code_from = Some("image name");
            }
        }
        u.code.dedup();
    }

    for u in &mut units {
        u.file = dotted(&u.file);
    }
    units.sort_by(|a, b| {
        a.platform
            .cmp(&b.platform)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
    });
    let truncated = units.len() > MAX_UNITS;
    units.truncate(MAX_UNITS);
    Topology {
        files_scanned,
        config_files: config_files.into_iter().collect(),
        skipped_example_files: 0,
        units,
        truncated,
    }
}

/// Plain-text report.
pub fn render_text(t: &Topology) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    if t.units.is_empty() {
        out.push_str("No deployment configuration found");
        if t.skipped_example_files > 0 {
            let _ = write!(
                out,
                " ({} file(s) in test, example or docs folders were skipped)",
                t.skipped_example_files
            );
        }
        out.push_str(".\n");
        return out;
    }
    let _ = writeln!(out, "What the deployment configuration says will run\n");
    let mut platform = "";
    for u in &t.units {
        if u.platform != platform {
            platform = &u.platform;
            let _ = writeln!(out, "{platform}");
        }
        let mut head = format!("  {} {}  ({}:{}", u.kind, u.name, u.file, u.line);
        if let Some(ns) = &u.namespace {
            let _ = write!(head, ", namespace {ns}");
        }
        head.push(')');
        let _ = writeln!(out, "{head}");
        let mut facts = Vec::new();
        if !u.images.is_empty() {
            facts.push(format!("image {}", u.images.join(", ")));
        }
        if let Some(r) = &u.runtime {
            facts.push(r.clone());
        }
        if let Some(h) = &u.handler {
            facts.push(format!("handler {h}"));
        }
        if let Some(n) = u.replicas {
            facts.push(format!("{n} replica{}", if n == 1 { "" } else { "s" }));
        }
        if let Some(a) = &u.autoscale {
            facts.push(format!("autoscaled {a}"));
        }
        if let Some(s) = &u.schedule {
            facts.push(format!("runs on {s}"));
        }
        if !u.ports.is_empty() {
            facts.push(format!("port {}", u.ports.join(", ")));
        }
        if !facts.is_empty() {
            let _ = writeln!(out, "    {}", facts.join(" · "));
        }
        if let Some(c) = &u.command {
            let _ = writeln!(out, "    runs {c}");
        }
        if !u.triggers.is_empty() {
            let _ = writeln!(out, "    reached by {}", u.triggers.join(" · "));
        }
        if !u.depends_on.is_empty() {
            let _ = writeln!(out, "    starts after {}", u.depends_on.join(", "));
        }
        if !u.code.is_empty() {
            let _ = writeln!(
                out,
                "    code {} (from {})",
                u.code.join(", "),
                u.code_from.unwrap_or("config")
            );
        }
    }
    let _ = write!(
        out,
        "\n{} unit(s) in {} file(s).",
        t.units.len(),
        t.config_files.len()
    );
    if t.truncated {
        let _ = write!(out, " Showing the first {MAX_UNITS}.");
    }
    if t.skipped_example_files > 0 {
        let _ = write!(
            out,
            " Skipped {} file(s) in test, example or docs folders.",
            t.skipped_example_files
        );
    }
    out.push_str(" Read from configuration only; nothing was run or deployed.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(files: &[(&str, &str)]) -> (tempfile::TempDir, Topology) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let t = topology(dir.path());
        (dir, t)
    }

    fn named<'t>(t: &'t Topology, name: &str) -> Vec<&'t Unit> {
        t.units.iter().filter(|u| u.name == name).collect()
    }

    const DEPLOYMENT: &str = "apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
spec:
  replicas: 2
  selector:
    matchLabels: {app: api}
  template:
    metadata:
      labels: {app: api}
    spec:
      containers:
        - name: api
          image: registry.example.com/team/api:1.0
          ports: [{containerPort: 8080}]
---
apiVersion: v1
kind: Service
metadata:
  name: api
spec:
  selector: {app: api}
  ports: [{port: 80, targetPort: 8080}]
";

    #[test]
    fn links_services_and_image_code() {
        let (_d, t) = repo(&[
            ("deploy/api.yaml", DEPLOYMENT),
            ("services/api/Dockerfile", "FROM scratch\n"),
            ("services/api/main.go", "package main\n"),
        ]);
        let api = named(&t, "api");
        assert_eq!(api.len(), 1, "{:?}", t.units);
        let u = api[0];
        assert_eq!(u.platform, "Kubernetes");
        assert_eq!(u.replicas, Some(2));
        assert_eq!(u.triggers.len(), 1);
        assert!(u.triggers[0].contains("Service api"), "{:?}", u.triggers);
        assert_eq!(u.code, vec!["./services/api/".to_string()]);
    }

    #[test]
    fn copies_of_manifests_link_within_their_own_folder() {
        let (_d, t) = repo(&[
            ("plain/api.yaml", DEPLOYMENT),
            ("bundle/all.yaml", DEPLOYMENT),
        ]);
        let api = named(&t, "api");
        assert_eq!(api.len(), 2);
        for u in api {
            assert_eq!(u.triggers.len(), 1, "{:?}", u.triggers);
        }
    }

    #[test]
    fn helm_selectors_match_templated_labels() {
        let template = "apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ .Values.name }}
spec:
  selector:
    matchLabels:
      app: {{ .Values.name }}
  template:
    metadata:
      labels:
        app: {{.Values.name}}
    spec:
      containers:
        - name: web
          image: \"{{ .Values.image }}\"
---
apiVersion: v1
kind: Service
metadata:
  name: {{ .Values.name }}
spec:
  selector:
    app: {{- .Values.name }}
  ports:
    - port: 80
";
        let (_d, t) = repo(&[
            ("chart/Chart.yaml", "name: web\nversion: 1.0.0\n"),
            ("chart/templates/web.yaml", template),
        ]);
        assert_eq!(t.units.len(), 1, "{:?}", t.units);
        let u = &t.units[0];
        assert!(u.platform.starts_with("Helm chart"), "{}", u.platform);
        assert_eq!(u.name, "{{ .Values.name }}");
        assert_eq!(u.triggers.len(), 1, "{:?}", u.triggers);
    }

    #[test]
    fn reads_serverless_functions_and_handlers() {
        let (_d, t) = repo(&[
            (
                "svc/serverless.yml",
                "service: jobs
provider:
  name: aws
  runtime: nodejs20.x
functions:
  create:
    handler: src/api.create
    events:
      - httpApi:
          method: POST
          path: /jobs
",
            ),
            ("svc/src/api.ts", "export const create = 1;\n"),
        ]);
        let u = named(&t, "create")[0];
        assert_eq!(u.runtime.as_deref(), Some("nodejs20.x"));
        assert_eq!(u.triggers, vec!["HTTP POST /jobs".to_string()]);
        assert_eq!(u.code, vec!["./svc/src/api.ts".to_string()]);
    }

    #[test]
    fn reads_sam_functions() {
        let (_d, t) = repo(&[
            (
                "template.yaml",
                "Transform: AWS::Serverless-2016-10-31
Resources:
  Hello:
    Type: AWS::Serverless::Function
    Properties:
      CodeUri: hello/
      Handler: app.handler
      Runtime: python3.12
      Events:
        Api:
          Type: Api
          Properties:
            Path: /hello
            Method: get
",
            ),
            ("hello/app.py", "def handler(e, c): pass\n"),
        ]);
        let u = named(&t, "Hello")[0];
        assert_eq!(u.platform, "AWS SAM");
        assert_eq!(u.triggers, vec!["HTTP GET /hello".to_string()]);
        assert_eq!(u.code, vec!["./hello/app.py".to_string()]);
    }

    #[test]
    fn compose_build_contexts_point_at_code() {
        let (_d, t) = repo(&[
            (
                "docker-compose.yml",
                "services:\n  web:\n    build: ./web\n    ports: [\"8080:80\"]\n  db:\n    image: postgres:16\n",
            ),
            ("web/Dockerfile", "FROM scratch\n"),
        ]);
        assert_eq!(named(&t, "web")[0].code, vec!["./web/".to_string()]);
        assert_eq!(named(&t, "db")[0].images, vec!["postgres:16".to_string()]);
    }

    #[test]
    fn compose_depends_on_lists_and_maps() {
        let (_d, t) = repo(&[(
            "compose.yaml",
            "services:\n  web:\n    image: web\n    depends_on: [db, cache]\n  worker:\n    image: worker\n    depends_on:\n      db:\n        condition: service_healthy\n  db:\n    image: postgres:16\n  cache:\n    image: redis\n",
        )]);
        let deps = |n| named(&t, n)[0].depends_on.clone();
        assert_eq!(deps("web"), vec!["db".to_string(), "cache".to_string()]);
        assert_eq!(deps("worker"), vec!["db".to_string()]);
        assert!(deps("db").is_empty());
        assert!(render_text(&t).contains("starts after db, cache"));
    }

    #[test]
    fn example_folders_are_skipped_and_counted() {
        let (_d, t) = repo(&[
            ("examples/api.yaml", DEPLOYMENT),
            ("test/fixtures/other.yaml", "a: 1\n"),
        ]);
        assert!(t.units.is_empty());
        assert_eq!(t.skipped_example_files, 1);
        assert!(render_text(&t).contains("1 file(s)"));
    }

    #[test]
    fn hostile_yaml_does_not_break_the_scan() {
        let bomb = "a: &a [x, x, x, x, x, x, x, x, x, x]\nb: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a, *a]\nc: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b, *b]\nkind: Deployment\n";
        let (_d, t) = repo(&[("bomb.yaml", bomb), ("deploy/api.yaml", DEPLOYMENT)]);
        assert_eq!(named(&t, "api").len(), 1);
    }
}
