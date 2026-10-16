//! How to run a project.
//!
//! Gathers what a newcomer needs to start working on a repository: the
//! commands it defines (package.json scripts, Make and just targets, Docker,
//! Compose services, Cargo binaries, Python entry points, Procfile), the
//! environment variables it reads and the HTTP routes it serves.
//!
//! Everything is read, never run. Environment variables are reported by name
//! only: `.env` files are never opened, example files such as `.env.example`
//! contribute only the text before `=` on each line, and no value from code is
//! kept. A command that looks like it contains a credential is hidden.
//! Routes come from matching common framework patterns, so the list can miss
//! routes or include a few that are not.

use crate::analysis::dotted;
use crate::safe_text::sanitize;
use rayon::prelude::*;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Most commands returned.
pub const MAX_COMMANDS: usize = 200;
/// Most environment variables returned.
pub const MAX_ENV_VARS: usize = 300;
/// Most routes returned.
pub const MAX_ROUTES: usize = 300;
/// Most commands taken from one file.
const PER_FILE_COMMANDS: usize = 50;
/// Longest text shown for a command, script or route path.
const MAX_TEXT: usize = 160;
/// Most files listed for one environment variable.
const MAX_ENV_FILES: usize = 5;
/// Longer lines are skipped: they are minified or generated.
const MAX_LINE: usize = 2_000;

/// One way to run or build something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunCommand {
    /// File that defines it, relative to the scanned folder.
    pub file: String,
    /// `npm`, `make`, `just`, `docker`, `compose`, `cargo`, `python` or `procfile`.
    pub tool: &'static str,
    pub name: String,
    /// What to type, from the folder that holds `file`.
    pub run: String,
    /// What it does, when the file says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One environment variable, by name only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvVar {
    pub name: String,
    /// Listed in an example file such as `.env.example`.
    pub documented: bool,
    /// How many times code reads it.
    pub references: usize,
    /// Up to five files that mention it.
    pub files: Vec<String>,
}

/// One HTTP route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    /// `GET`, `POST`, …, `ANY` when every method is accepted or the method
    /// could not be read, `CRUD` for a resource declaration.
    pub method: String,
    pub path: String,
    pub file: String,
    pub line: usize,
}

/// Everything found.
#[derive(Debug, Clone, Serialize)]
pub struct Runbook {
    pub files_scanned: usize,
    pub commands: Vec<RunCommand>,
    pub env_vars: Vec<EnvVar>,
    pub routes: Vec<Route>,
    /// `true` when a list was cut to its limit.
    pub truncated: bool,
}

/// What one file contributed.
#[derive(Debug, Default)]
struct Found {
    file: String,
    commands: Vec<RunCommand>,
    /// Variable names and whether they came from an example file.
    env: Vec<(String, bool)>,
    routes: Vec<Route>,
}

/// Read every file under `root` that the scanner would read.
pub fn runbook(root: &Path) -> Runbook {
    let files = crate::scanner::scan_repository(root);
    let relative: Vec<String> = files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(root).unwrap_or(path);
            sanitize(&relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let all: BTreeSet<&str> = relative.iter().map(String::as_str).collect();
    let found: Vec<Found> = files
        .par_iter()
        .zip(relative.par_iter())
        .filter(|(_, file)| kind_of(file).is_some())
        .filter_map(|(path, file)| {
            let bytes = std::fs::read(path).ok()?;
            Some(read_file(file, &String::from_utf8_lossy(&bytes), &all))
        })
        .collect();
    finish(files.len(), found)
}

fn finish(files_scanned: usize, found: Vec<Found>) -> Runbook {
    let mut commands = Vec::new();
    let mut routes = Vec::new();
    let mut env: BTreeMap<String, EnvVar> = BTreeMap::new();
    for f in found {
        commands.extend(f.commands);
        routes.extend(f.routes);
        for (name, documented) in f.env {
            let var = env.entry(name.clone()).or_insert_with(|| EnvVar {
                name,
                documented: false,
                references: 0,
                files: Vec::new(),
            });
            if documented {
                var.documented = true;
            } else {
                var.references += 1;
            }
            if var.files.len() < MAX_ENV_FILES && !var.files.contains(&f.file) {
                var.files.push(f.file.clone());
            }
        }
    }
    for c in &mut commands {
        c.file = dotted(&c.file);
    }
    for r in &mut routes {
        r.file = dotted(&r.file);
    }
    commands.sort_by(|a, b| a.file.cmp(&b.file));
    routes.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
    let mut env_vars: Vec<EnvVar> = env.into_values().collect();
    for var in &mut env_vars {
        for file in &mut var.files {
            *file = dotted(file);
        }
        var.files.sort();
    }
    let truncated =
        commands.len() > MAX_COMMANDS || env_vars.len() > MAX_ENV_VARS || routes.len() > MAX_ROUTES;
    commands.truncate(MAX_COMMANDS);
    env_vars.truncate(MAX_ENV_VARS);
    routes.truncate(MAX_ROUTES);
    Runbook {
        files_scanned,
        commands,
        env_vars,
        routes,
        truncated,
    }
}

// ---------------------------------------------------------------------------
// Which files are read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Package,
    Make,
    Just,
    Docker,
    Compose,
    Cargo,
    PyProject,
    DjangoManage,
    Procfile,
    EnvExample,
    Code,
}

/// Extensions read for environment variables and routes.
#[rustfmt::skip]
const CODE_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "py", "rb", "php", "go", "rs", "java",
    "kt", "cs", "vue", "svelte",
];

/// How a file is read, or `None` when it is not read at all. `.env` and
/// other files holding real values never match.
fn kind_of(file: &str) -> Option<Kind> {
    let name = file_name(file);
    let lower = name.to_ascii_lowercase();
    let kind = match name {
        "package.json" => Kind::Package,
        "Makefile" | "makefile" | "GNUmakefile" => Kind::Make,
        "justfile" | "Justfile" | ".justfile" => Kind::Just,
        "Cargo.toml" => Kind::Cargo,
        "pyproject.toml" => Kind::PyProject,
        "manage.py" => Kind::DjangoManage,
        "Procfile" => Kind::Procfile,
        _ if name == "Dockerfile" || name.starts_with("Dockerfile.") => Kind::Docker,
        _ if lower.ends_with(".dockerfile") => Kind::Docker,
        _ if is_compose(&lower) => Kind::Compose,
        _ if is_env_example(&lower) => Kind::EnvExample,
        _ if CODE_EXTENSIONS.contains(&extension(&lower)) => Kind::Code,
        _ => return None,
    };
    Some(kind)
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

/// Example environment files: documentation, never real values.
fn is_env_example(lower: &str) -> bool {
    const SUFFIXES: &[&str] = &["example", "sample", "template", "dist", "defaults"];
    if matches!(
        lower,
        "env.example" | "example.env" | "env.sample" | "sample.env"
    ) {
        return true;
    }
    lower.starts_with(".env")
        && lower
            .rsplit_once('.')
            .is_some_and(|(_, last)| SUFFIXES.contains(&last))
}

fn file_name(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Folder of a relative path, with a trailing `/` unless it is the root.
fn folder(file: &str) -> &str {
    file.rfind('/').map_or("", |i| &file[..=i])
}

fn read_file(file: &str, text: &str, all: &BTreeSet<&str>) -> Found {
    let mut found = Found {
        file: file.to_string(),
        ..Found::default()
    };
    let Some(kind) = kind_of(file) else {
        return found;
    };
    let mut commands = match kind {
        Kind::Package => package_scripts(file, text, all),
        Kind::Make => make_targets(file, text),
        Kind::Just => just_recipes(file, text),
        Kind::Docker => dockerfile(file, text),
        Kind::Compose => compose_services(file, text),
        Kind::Cargo => cargo_binaries(file, text, all),
        Kind::PyProject => python_scripts(file, text),
        Kind::DjangoManage => vec![command(
            file,
            "python",
            "runserver",
            "python manage.py runserver".to_string(),
            Some("Django development server"),
        )],
        Kind::Procfile => procfile(file, text),
        Kind::EnvExample => {
            found.env = example_names(text).into_iter().map(|n| (n, true)).collect();
            Vec::new()
        }
        Kind::Code => {
            found.env = code_env_names(text)
                .into_iter()
                .map(|n| (n, false))
                .collect();
            found.routes = routes(file, text);
            Vec::new()
        }
    };
    commands.truncate(PER_FILE_COMMANDS);
    found.commands = commands;
    found
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Sanitize and shorten text taken from the repository.
fn clip(s: &str) -> String {
    let s = sanitize(s.trim());
    if s.chars().count() <= MAX_TEXT {
        return s;
    }
    let mut short: String = s.chars().take(MAX_TEXT).collect();
    short.push('…');
    short
}

fn command(
    file: &str,
    tool: &'static str,
    name: &str,
    run: String,
    detail: Option<&str>,
) -> RunCommand {
    let hide = |s: &str| {
        if crate::audit::find_secrets(s).is_empty() {
            clip(s)
        } else {
            "(hidden: looks like it contains a credential)".to_string()
        }
    };
    RunCommand {
        file: file.to_string(),
        tool,
        name: hide(name),
        run: hide(&run),
        detail: detail.map(str::trim).filter(|d| !d.is_empty()).map(hide),
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Whether `name` is a plausible environment variable name.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 100
}

/// The string literal at the start of `s` (after spaces) and the text after
/// it. A Python `r` prefix is allowed.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let s = s
        .strip_prefix(['r', 'R'])
        .filter(|r| r.starts_with(['"', '\'']))
        .unwrap_or(s);
    let quote = s.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let body = &s[1..];
    let end = body.find(quote)?;
    Some((&body[..end], &body[end + 1..]))
}

/// The identifier that ends at byte `end` of `line`.
fn ident_before(line: &str, end: usize) -> &str {
    let start = line[..end]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_char(*c))
        .last()
        .map_or(end, |(i, _)| i);
    &line[start..end]
}

/// Whether `needle` starts at `at` without an identifier character before it.
fn at_boundary(line: &str, at: usize) -> bool {
    !line[..at].chars().next_back().is_some_and(is_ident_char)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn package_scripts(file: &str, text: &str, all: &BTreeSet<&str>) -> Vec<RunCommand> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let dir = folder(file);
    let has = |lock: &str| all.contains(format!("{dir}{lock}").as_str());
    let runner = if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else if has("bun.lockb") || has("bun.lock") {
        "bun"
    } else {
        "npm"
    };
    scripts
        .iter()
        .filter_map(|(name, body)| {
            Some(command(
                file,
                "npm",
                name,
                format!("{runner} run {name}"),
                Some(body.as_str()?),
            ))
        })
        .collect()
}

fn make_targets(file: &str, text: &str) -> Vec<RunCommand> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '.']) {
            continue;
        }
        let Some((targets, rest)) = line.split_once(':') else {
            continue;
        };
        if rest.starts_with(['=', ':']) || targets.contains(['=', '$', '%']) {
            continue;
        }
        let help = rest.split_once("##").map(|(_, h)| h);
        for target in targets.split_whitespace() {
            let valid = target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
            if valid && seen.insert(target) {
                out.push(command(
                    file,
                    "make",
                    target,
                    format!("make {target}"),
                    help,
                ));
            }
        }
    }
    out
}

fn just_recipes(file: &str, text: &str) -> Vec<RunCommand> {
    const KEYWORDS: &[&str] = &["set", "alias", "import", "mod", "export"];
    let mut out = Vec::new();
    let mut comment: Option<&str> = None;
    for line in text.lines() {
        if let Some(c) = line.strip_prefix('#') {
            comment = Some(c);
            continue;
        }
        let doc = comment.take();
        if line.starts_with([' ', '\t', '[']) || line.contains(":=") {
            continue;
        }
        let Some((head, _)) = line.split_once(':') else {
            continue;
        };
        let name = head.trim_start_matches('@');
        let name = name.split_whitespace().next().unwrap_or("");
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if valid && !KEYWORDS.contains(&name) {
            out.push(command(file, "just", name, format!("just {name}"), doc));
        }
    }
    out
}

fn dockerfile(file: &str, text: &str) -> Vec<RunCommand> {
    let mut ports = Vec::new();
    let mut start = None;
    for line in text.lines() {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match word.to_ascii_uppercase().as_str() {
            "EXPOSE" => ports.extend(rest.split_whitespace()),
            "CMD" | "ENTRYPOINT" => start = Some(rest.trim()),
            _ => {}
        }
    }
    let name = file_name(file);
    let run = if name == "Dockerfile" {
        "docker build .".to_string()
    } else {
        format!("docker build -f {name} .")
    };
    let mut detail = Vec::new();
    if !ports.is_empty() {
        detail.push(format!("exposes {}", ports.join(", ")));
    }
    if let Some(start) = start {
        detail.push(format!("starts {start}"));
    }
    vec![command(
        file,
        "docker",
        "image",
        run,
        Some(detail.join("; ").as_str()),
    )]
}

fn compose_services(file: &str, text: &str) -> Vec<RunCommand> {
    let name = file_name(file);
    let default = matches!(
        name,
        "compose.yaml" | "compose.yml" | "docker-compose.yml" | "docker-compose.yaml"
    );
    let mut services: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut in_services = false;
    let mut indent = None;
    let mut in_ports = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let level = line.len() - line.trim_start().len();
        if level == 0 {
            in_services = trimmed == "services:";
            indent = None;
            continue;
        }
        if !in_services {
            continue;
        }
        let service_level = *indent.get_or_insert(level);
        if level == service_level {
            in_ports = false;
            if let Some(service) = trimmed.strip_suffix(':') {
                let valid = service
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
                if valid {
                    services.push((service, Vec::new()));
                }
            }
        } else if let Some(item) = trimmed.strip_prefix("- ") {
            if in_ports {
                if let Some((_, ports)) = services.last_mut() {
                    ports.push(item.trim().trim_matches(['"', '\'']));
                }
            }
        } else {
            in_ports = trimmed == "ports:";
        }
    }
    services
        .into_iter()
        .map(|(service, ports)| {
            let run = if default {
                format!("docker compose up {service}")
            } else {
                format!("docker compose -f {name} up {service}")
            };
            let detail = (!ports.is_empty()).then(|| format!("ports {}", ports.join(", ")));
            command(file, "compose", service, run, detail.as_deref())
        })
        .collect()
}

fn cargo_binaries(file: &str, text: &str, all: &BTreeSet<&str>) -> Vec<RunCommand> {
    let Ok(table) = toml::from_str::<toml::Table>(text) else {
        return Vec::new();
    };
    let Some(package) = table.get("package") else {
        return Vec::new();
    };
    let bins: Vec<&str> = table
        .get("bin")
        .and_then(|b| b.as_array())
        .map(|bins| {
            bins.iter()
                .filter_map(|b| b.get("name")?.as_str())
                .collect()
        })
        .unwrap_or_default();
    if bins.len() > 1 {
        return bins
            .into_iter()
            .map(|bin| command(file, "cargo", bin, format!("cargo run --bin {bin}"), None))
            .collect();
    }
    let has_main = all.contains(format!("{}src/main.rs", folder(file)).as_str());
    if bins.is_empty() && !has_main {
        return Vec::new();
    }
    let name = package
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("run");
    vec![command(file, "cargo", name, "cargo run".to_string(), None)]
}

fn python_scripts(file: &str, text: &str) -> Vec<RunCommand> {
    let Ok(table) = toml::from_str::<toml::Table>(text) else {
        return Vec::new();
    };
    let tables = [
        table.get("project").and_then(|p| p.get("scripts")),
        table
            .get("tool")
            .and_then(|t| t.get("poetry"))
            .and_then(|p| p.get("scripts")),
    ];
    tables
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_table())
        .flat_map(|t| t.iter())
        .map(|(name, target)| command(file, "python", name, name.clone(), target.as_str()))
        .collect()
}

fn procfile(file: &str, text: &str) -> Vec<RunCommand> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(|(name, cmd)| command(file, "procfile", name, cmd.trim().to_string(), None))
        .collect()
}

// ---------------------------------------------------------------------------
// Environment variables
// ---------------------------------------------------------------------------

/// Names in an example file: the text before `=`; values are dropped.
fn example_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (name, _) = line.split_once('=')?;
            let name = name.trim();
            is_env_name(name).then(|| name.to_string())
        })
        .collect()
}

/// Accessors followed by a bare name, as in `process.env.PORT`.
const ENV_PROPERTIES: &[&str] = &["process.env.", "import.meta.env.", "Bun.env."];

/// Accessors followed by a quoted name, as in `os.getenv("PORT")`.
#[rustfmt::skip]
const ENV_CALLS: &[&str] = &[
    "process.env[", "getenv(", "Getenv(", "LookupEnv(", "environ[", "environ.get(", "env::var(",
    "env::var_os(", "env!(", "option_env!(", "ENV[", "ENV.fetch(", "_ENV[",
    "GetEnvironmentVariable(", "Deno.env.get(",
];

/// Variable names read by code, one entry per read. Only names are kept.
fn code_env_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.len() <= MAX_LINE) {
        for needle in ENV_PROPERTIES {
            for (at, _) in line.match_indices(needle) {
                let rest = &line[at + needle.len()..];
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                let name = &rest[..len];
                if is_env_name(name) && !rest[len..].starts_with('(') {
                    out.push(name.to_string());
                }
            }
        }
        for needle in ENV_CALLS {
            for (at, _) in line.match_indices(needle) {
                if !at_boundary(line, at) {
                    continue;
                }
                if let Some((name, _)) = quoted(&line[at + needle.len()..]) {
                    if is_env_name(name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "head", "options", "all", "any",
];

/// Receivers whose lowercase `.get("/…")` calls declare routes rather than
/// make requests.
const SERVER_RECEIVERS: &[&str] = &[
    "app", "router", "server", "fastify", "r", "e", "g", "mux", "route", "routes", "v1", "v2",
];

/// Spring and NestJS annotations with the method they declare.
#[rustfmt::skip]
const SPRING: &[(&str, &str)] = &[
    ("@GetMapping", "GET"), ("@PostMapping", "POST"), ("@PutMapping", "PUT"),
    ("@PatchMapping", "PATCH"), ("@DeleteMapping", "DELETE"), ("@RequestMapping", "ANY"),
];
#[rustfmt::skip]
const NEST: &[(&str, &str)] = &[
    ("@Get", "GET"), ("@Post", "POST"), ("@Put", "PUT"), ("@Patch", "PATCH"),
    ("@Delete", "DELETE"), ("@Head", "HEAD"), ("@Options", "OPTIONS"), ("@All", "ANY"),
];

fn routes(file: &str, text: &str) -> Vec<Route> {
    let name = file_name(file);
    let ext = extension(name);
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<(usize, String, String)> = Vec::new();
    let mut prefix = String::new();
    for (i, line) in lines.iter().enumerate() {
        if line.len() > MAX_LINE {
            continue;
        }
        let joined = continued(&lines, i);
        let line: &str = &joined;
        let mut push =
            |method: &str, path: &str| out.push((i + 1, method.to_string(), path.to_string()));
        match ext {
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => {
                for (m, p) in method_calls(line) {
                    push(&m, p);
                }
                if let Some(p) = annotation(line, "@Controller") {
                    prefix = p.to_string();
                }
                for (tag, method) in NEST {
                    if let Some(p) = annotation(line, tag) {
                        push(method, &join_path(&prefix, p));
                    }
                }
            }
            "go" => {
                for (m, p) in method_calls(line) {
                    push(&m, p);
                }
                for (m, p) in go_handlers(line) {
                    push(m, p);
                }
            }
            "py" => {
                if let Some((m, p)) = python_decorator(line) {
                    push(&m, p);
                }
                if name == "urls.py" {
                    for p in django_paths(line) {
                        push("ANY", &join_path("", p));
                    }
                }
            }
            "rs" => {
                for (m, p) in rust_routes(line) {
                    push(&m, p);
                }
            }
            "java" | "kt" => {
                for (tag, method) in SPRING {
                    let Some(p) = annotation(line, tag) else {
                        continue;
                    };
                    let class_level = *tag == "@RequestMapping"
                        && lines[i..].iter().take(5).any(|l| l.contains("class "));
                    if class_level {
                        prefix = p.to_string();
                    } else {
                        push(method, &join_path(&prefix, p));
                    }
                }
            }
            "php" => {
                for (m, p) in laravel(line) {
                    push(&m, &join_path("", p));
                }
            }
            "rb" if name == "routes.rb" => {
                if let Some((m, p)) = rails(line) {
                    push(m, &join_path("", p));
                }
            }
            _ => {}
        }
    }
    if let Some((path, methods)) = next_route(file, &lines) {
        for (line, method) in methods {
            out.push((line, method, path.clone()));
        }
    }
    out.into_iter()
        .map(|(line, method, path)| Route {
            method,
            path: clip(&path),
            file: file.to_string(),
            line,
        })
        .collect()
}

/// A line ending in an open call, joined to the next non-empty line so
/// `router.get(` followed by `'/x',` reads as one call.
fn continued<'a>(lines: &[&'a str], i: usize) -> Cow<'a, str> {
    let line = lines[i];
    if !line.trim_end().ends_with('(') {
        return Cow::Borrowed(line);
    }
    match lines[i + 1..]
        .iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
    {
        Some(next) if next.len() <= MAX_LINE => Cow::Owned(format!("{}{next}", line.trim_end())),
        _ => Cow::Borrowed(line),
    }
}

/// `/a` + `b/` → `/a/b`.
fn join_path(prefix: &str, path: &str) -> String {
    let parts: Vec<&str> = [prefix, path]
        .iter()
        .flat_map(|p| p.split('/'))
        .filter(|p| !p.is_empty())
        .collect();
    format!("/{}", parts.join("/"))
}

/// `app.get("/x")`, `router.post('/x')`, gin `r.GET("/x")`, chi `r.Get("/x")`.
fn method_calls(line: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    for (dot, _) in line.match_indices('.') {
        let after = &line[dot + 1..];
        let len = after
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(after.len());
        let name = &after[..len];
        let lower = name.to_ascii_lowercase();
        if !after[len..].starts_with('(') || !HTTP_METHODS.contains(&lower.as_str()) {
            continue;
        }
        let receiver = ident_before(line, dot);
        let server = name != lower
            || SERVER_RECEIVERS.contains(&receiver)
            || ["router", "Router", "routes", "Routes", "app", "App"]
                .iter()
                .any(|s| receiver.ends_with(s));
        if receiver.is_empty() || !server {
            continue;
        }
        if let Some((path, _)) = quoted(&after[len + 1..]) {
            if path.starts_with('/') {
                out.push((lower.to_ascii_uppercase(), path));
            }
        }
    }
    out
}

/// The path in `@Tag("/x")`, `@Tag(value = "/x")` or `@Tag()`. `None` when
/// the line has no such annotation; `@Getter` does not match `@Get`.
fn annotation<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let at = line.find(tag)?;
    let rest = &line[at + tag.len()..];
    if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let Some(args) = rest.strip_prefix('(') else {
        return Some("");
    };
    let args = &args[..args.find(')').unwrap_or(args.len())];
    let Some(start) = args.find(['"', '\'']) else {
        return Some("");
    };
    Some(quoted(&args[start..]).map_or("", |(p, _)| p))
}

/// net/http `HandleFunc("/x", …)` and Go 1.22 `HandleFunc("GET /x", …)`.
fn go_handlers(line: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for needle in ["HandleFunc(", "Handle("] {
        for (at, _) in line.match_indices(needle) {
            if !at_boundary(line, at) {
                continue;
            }
            let Some((pattern, _)) = quoted(&line[at + needle.len()..]) else {
                continue;
            };
            let (method, path) = match pattern.split_once(' ') {
                Some((m, p)) => (m, p.trim_start()),
                None => ("ANY", pattern),
            };
            if path.starts_with('/') {
                out.push((method, path));
            }
        }
    }
    out
}

/// Flask, FastAPI, Quart and Sanic: `@app.get("/x")`, `@bp.route("/x",
/// methods=["POST"])`.
fn python_decorator(line: &str) -> Option<(String, &str)> {
    let decorator = line.trim_start().strip_prefix('@')?;
    let paren = decorator.find('(')?;
    let callee = &decorator[..paren];
    if !callee.contains('.') || !callee.chars().all(|c| is_ident_char(c) || c == '.') {
        return None;
    }
    let method = callee.rsplit('.').next()?;
    let (path, rest) = quoted(&decorator[paren + 1..])?;
    if !path.starts_with('/') {
        return None;
    }
    let method = match method {
        "route" | "api_route" => methods_list(rest).unwrap_or_else(|| "GET".to_string()),
        "websocket" => "WS".to_string(),
        m if HTTP_METHODS.contains(&m) => m.to_ascii_uppercase(),
        _ => return None,
    };
    Some((method, path))
}

/// `methods=["GET", "POST"]` → `GET,POST`.
fn methods_list(rest: &str) -> Option<String> {
    let after = &rest[rest.find("methods")? + "methods".len()..];
    let open = after.find(['[', '('])?;
    let inner = &after[open + 1..];
    let inner = &inner[..inner.find([']', ')'])?];
    let methods: Vec<String> = inner
        .split(',')
        .map(|m| m.trim().trim_matches(['"', '\'']).to_ascii_uppercase())
        .filter(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_alphabetic()))
        .collect();
    (!methods.is_empty()).then(|| methods.join(","))
}

/// Django `path("users/", …)` and `re_path(r"^users/$", …)`.
fn django_paths(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for needle in ["path(", "re_path("] {
        for (at, _) in line.match_indices(needle) {
            if !at_boundary(line, at) {
                continue;
            }
            if let Some((path, _)) = quoted(&line[at + needle.len()..]) {
                out.push(path.trim_start_matches('^').trim_end_matches('$'));
            }
        }
    }
    out
}

/// actix and Rocket `#[get("/x")]`, axum `.route("/x", get(h).post(h))`.
fn rust_routes(line: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let trimmed = line.trim_start();
    if let Some(attr) = trimmed.strip_prefix("#[") {
        if let Some((name, args)) = attr.split_once('(') {
            if HTTP_METHODS.contains(&name) {
                if let Some((path, _)) = quoted(args) {
                    out.push((name.to_ascii_uppercase(), path));
                }
            }
        }
    }
    for (at, _) in line.match_indices(".route(") {
        let Some((path, rest)) = quoted(&line[at + ".route(".len()..]) else {
            continue;
        };
        if !path.starts_with('/') {
            continue;
        }
        let mut methods: Vec<String> = Vec::new();
        for method in HTTP_METHODS {
            let call = format!("{method}(");
            let found = rest.match_indices(&call).any(|(i, _)| at_boundary(rest, i));
            if found {
                methods.push(method.to_ascii_uppercase());
            }
        }
        let method = if methods.is_empty() {
            "ANY".to_string()
        } else {
            methods.join(",")
        };
        out.push((method, path));
    }
    out
}

/// Laravel `Route::get('/x', …)` and `Route::resource('photos', …)`.
fn laravel(line: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    for (at, _) in line.match_indices("Route::") {
        let after = &line[at + "Route::".len()..];
        let len = after
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(after.len());
        let name = &after[..len].to_ascii_lowercase();
        let method = match name.as_str() {
            "resource" | "apiresource" => "CRUD".to_string(),
            "match" => continue,
            m if HTTP_METHODS.contains(&m) => m.to_ascii_uppercase(),
            _ => continue,
        };
        let Some(args) = after[len..].strip_prefix('(') else {
            continue;
        };
        if let Some((path, _)) = quoted(args) {
            out.push((method, path));
        }
    }
    out
}

/// Rails `get "/x"`, `post 'x'`, `root "home#index"` and `resources :users`.
fn rails(line: &str) -> Option<(&'static str, &str)> {
    let line = line.trim_start();
    let (word, rest) = line.split_once(char::is_whitespace)?;
    match word {
        "get" | "post" | "put" | "patch" | "delete" => {
            let method = match word {
                "get" => "GET",
                "post" => "POST",
                "put" => "PUT",
                "patch" => "PATCH",
                _ => "DELETE",
            };
            quoted(rest).map(|(path, _)| (method, path))
        }
        "root" => Some(("GET", "")),
        "resources" | "resource" => {
            let name = rest.trim_start().strip_prefix(':')?;
            let len = name
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(name.len());
            Some(("CRUD", &name[..len]))
        }
        _ => None,
    }
}

/// Next.js file routes: `app/**/route.ts` with exported method handlers, and
/// `pages/api/**`.
fn next_route(file: &str, lines: &[&str]) -> Option<(String, Vec<(usize, String)>)> {
    let parts: Vec<&str> = file.split('/').collect();
    let (last, dirs) = parts.split_last()?;
    let (stem, ext) = last.rsplit_once('.')?;
    if !matches!(ext, "ts" | "js" | "tsx" | "jsx" | "mts" | "mjs") {
        return None;
    }
    if let (Some(app), "route") = (dirs.iter().position(|d| *d == "app"), stem) {
        let segments: Vec<&str> = dirs[app + 1..]
            .iter()
            .copied()
            .filter(|d| !(d.starts_with('(') || d.starts_with('@')))
            .collect();
        let mut methods = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim_start().strip_prefix("export ") else {
                continue;
            };
            let rest = rest.trim_start_matches("async ").trim_start();
            let Some(rest) = rest
                .strip_prefix("function ")
                .or_else(|| rest.strip_prefix("const "))
            else {
                continue;
            };
            let len = rest
                .find(|c: char| !c.is_ascii_uppercase())
                .unwrap_or(rest.len());
            let name = &rest[..len];
            if HTTP_METHODS.contains(&name.to_ascii_lowercase().as_str()) {
                methods.push((i + 1, name.to_string()));
            }
        }
        return Some((join_path("", &segments.join("/")), methods));
    }
    let pages = dirs
        .windows(2)
        .position(|w| w[0] == "pages" && w[1] == "api")?;
    let mut segments: Vec<&str> = dirs[pages + 1..].to_vec();
    if stem != "index" {
        segments.push(stem);
    }
    Some((
        join_path("", &segments.join("/")),
        vec![(1, "ANY".to_string())],
    ))
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Plain text for the terminal.
pub fn render_text(book: &Runbook) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut out = String::from("\n");
    out.push_str(&format!(
        "  Runbook: {}, {}, {}\n\n",
        plural(book.commands.len(), "command"),
        plural(book.env_vars.len(), "environment variable"),
        plural(book.routes.len(), "route"),
    ));
    if book.commands.is_empty() && book.env_vars.is_empty() && book.routes.is_empty() {
        out.push_str("  No scripts, environment variables or routes found.\n\n");
        return out;
    }

    if !book.commands.is_empty() {
        out.push_str("  How to run (from the folder of each file)\n");
        let mut current = "";
        for c in &book.commands {
            if c.file != current {
                current = &c.file;
                out.push_str(&format!("\n    {current}\n"));
            }
            match &c.detail {
                Some(detail) => out.push_str(&format!("      {:<30} {detail}\n", c.run)),
                None => out.push_str(&format!("      {}\n", c.run)),
            }
        }
        out.push('\n');
    }

    if !book.env_vars.is_empty() {
        out.push_str("  Environment variables (names only; values are never read)\n\n");
        for v in &book.env_vars {
            let mut notes = Vec::new();
            if !v.documented {
                notes.push("not in an example file".to_string());
            }
            if v.references > 0 {
                notes.push(plural(v.references, "read"));
            }
            notes.push(v.files.join(", "));
            out.push_str(&format!("    {:<30} {}\n", v.name, notes.join(" · ")));
        }
        out.push('\n');
    }

    if !book.routes.is_empty() {
        out.push_str("  API routes\n\n");
        for r in &book.routes {
            out.push_str(&format!(
                "    {:<7} {:<36} {}:{}\n",
                r.method, r.path, r.file, r.line
            ));
        }
        out.push('\n');
    }

    if book.truncated {
        out.push_str("  Some lists were cut short; use --json for the capped totals.\n");
    }
    out.push_str("  Nothing was run. Routes come from common framework patterns and may\n");
    out.push_str("  be incomplete.\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, file: &str, body: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn set(files: &[&'static str]) -> BTreeSet<&'static str> {
        files.iter().copied().collect()
    }

    #[test]
    fn reads_package_scripts_with_the_right_runner() {
        let json = r#"{"scripts": {"dev": "next dev", "build": "next build"}}"#;
        let cmds = package_scripts("web/package.json", json, &set(&["web/pnpm-lock.yaml"]));
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].run, "pnpm run build");
        assert_eq!(cmds[1].detail.as_deref(), Some("next dev"));
        let cmds = package_scripts("package.json", json, &set(&[]));
        assert_eq!(cmds[0].run, "npm run build");
    }

    #[test]
    fn hides_scripts_with_credentials() {
        let token = format!("ghp_{}", "aB3".repeat(12));
        let json = format!(r#"{{"scripts": {{"push": "git push https://{token}@github.com/x"}}}}"#);
        let cmds = package_scripts("package.json", &json, &set(&[]));
        let detail = cmds[0].detail.as_deref().unwrap();
        assert!(!detail.contains("ghp_"));
        assert!(detail.contains("hidden"));
    }

    #[test]
    fn reads_make_targets() {
        let make = "VAR := 1\n.PHONY: build\nbuild: deps ## Build it\n\tcargo build\ntest lint:\n%.o: %.c\n";
        let cmds = make_targets("Makefile", make);
        let runs: Vec<&str> = cmds.iter().map(|c| c.run.as_str()).collect();
        assert_eq!(runs, ["make build", "make test", "make lint"]);
        assert_eq!(cmds[0].detail.as_deref(), Some("Build it"));
    }

    #[test]
    fn reads_just_recipes() {
        let just = "set shell := [\"bash\"]\n# Run tests\ntest *args:\n  cargo test\n@fmt:\n";
        let cmds = just_recipes("justfile", just);
        let runs: Vec<&str> = cmds.iter().map(|c| c.run.as_str()).collect();
        assert_eq!(runs, ["just test", "just fmt"]);
        assert_eq!(cmds[0].detail.as_deref(), Some("Run tests"));
    }

    #[test]
    fn reads_docker_and_compose() {
        let docker = "FROM node:20\nEXPOSE 3000\nCMD [\"node\", \"server.js\"]\n";
        let cmd = &dockerfile("Dockerfile", docker)[0];
        assert_eq!(cmd.run, "docker build .");
        assert!(cmd.detail.as_deref().unwrap().contains("exposes 3000"));

        let compose = "services:\n  web:\n    build: .\n    ports:\n      - \"3000:3000\"\n  db:\n    image: postgres\nvolumes:\n  data:\n";
        let cmds = compose_services("docker-compose.yml", compose);
        let runs: Vec<&str> = cmds.iter().map(|c| c.run.as_str()).collect();
        assert_eq!(runs, ["docker compose up web", "docker compose up db"]);
        assert_eq!(cmds[0].detail.as_deref(), Some("ports 3000:3000"));
    }

    #[test]
    fn reads_cargo_python_and_procfile() {
        let cargo = "[package]\nname = \"tool\"\n";
        let cmds = cargo_binaries("Cargo.toml", cargo, &set(&["src/main.rs"]));
        assert_eq!(cmds[0].run, "cargo run");
        assert!(cargo_binaries("Cargo.toml", cargo, &set(&[])).is_empty());

        let py = "[project.scripts]\nserve = \"app.cli:main\"\n";
        let cmds = python_scripts("pyproject.toml", py);
        assert_eq!(cmds[0].run, "serve");
        assert_eq!(cmds[0].detail.as_deref(), Some("app.cli:main"));

        let cmds = procfile("Procfile", "web: gunicorn app:app\n");
        assert_eq!(cmds[0].name, "web");
        assert_eq!(cmds[0].run, "gunicorn app:app");
    }

    #[test]
    fn env_names_never_keep_values() {
        let names =
            example_names("# comment\nDATABASE_URL=postgres://u:p@h/db\nexport PORT=3000\n");
        assert_eq!(names, ["DATABASE_URL", "PORT"]);

        let code = r#"const a = process.env.API_URL; os.getenv("SECRET_KEY"); std::env::var("RUST_LOG"); os.Getenv("ADDR"); ENV.fetch('REDIS_URL'); process.env["TOKEN"]"#;
        let names = code_env_names(code);
        assert_eq!(
            names,
            [
                "API_URL",
                "TOKEN",
                "SECRET_KEY",
                "ADDR",
                "RUST_LOG",
                "REDIS_URL"
            ]
        );
        assert!(code_env_names("MY_ENV[0]; process.env.hasOwnProperty(x)").is_empty());
    }

    #[test]
    fn real_env_files_are_never_read() {
        for name in [".env", ".env.local", ".env.production", "secrets.env"] {
            assert_eq!(kind_of(name), None, "{name}");
        }
        for name in [
            ".env.example",
            ".env.sample",
            ".env.local.example",
            "env.example",
        ] {
            assert_eq!(kind_of(name), Some(Kind::EnvExample), "{name}");
        }
    }

    #[test]
    fn finds_js_and_go_routes() {
        let routes = routes(
            "src/server.ts",
            "app.get('/users', h)\nrouter.post(\"/users/:id\", h)\naxios.get('/api')\nthis.http.get('/x')\n",
        );
        let found: Vec<(&str, &str)> = routes
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(found, [("GET", "/users"), ("POST", "/users/:id")]);
        assert_eq!(routes[1].line, 2);

        let go = routes_of("main.go", "r.GET(\"/ping\", h)\nhttp.HandleFunc(\"POST /items\", h)\nmux.Handle(\"/static/\", fs)\n");
        assert_eq!(go, ["GET /ping", "POST /items", "ANY /static/"]);
    }

    #[test]
    fn finds_routes_split_across_lines() {
        let ts = "router.get(\n  '/articles/feed',\n  auth.required,\n  handler,\n);\nrouter.post(\n\n  \"/articles\", h)\n";
        let found = routes("src/article.controller.ts", ts);
        let found: Vec<(usize, &str, &str)> = found
            .iter()
            .map(|r| (r.line, r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            found,
            [(1, "GET", "/articles/feed"), (6, "POST", "/articles")]
        );
    }

    /// Routes as `METHOD path`.
    fn routes_of(file: &str, text: &str) -> Vec<String> {
        routes(file, text)
            .into_iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect()
    }

    #[test]
    fn finds_decorator_and_annotation_routes() {
        let py = "@app.route('/login', methods=['GET', 'POST'])\n@router.get(\"/items/{id}\")\n@pytest.mark.parametrize('/x')\n";
        assert_eq!(
            routes_of("api.py", py),
            ["GET,POST /login", "GET /items/{id}"]
        );

        let nest = "@Controller('users')\nclass C {\n  @Get(':id')\n  @Post()\n  @Getter\n}\n";
        assert_eq!(
            routes_of("users.controller.ts", nest),
            ["GET /users/:id", "POST /users"]
        );

        let java = "@RestController\n@RequestMapping(\"/api\")\npublic class Api {\n  @GetMapping(\"/items\")\n  @PostMapping(value = \"/items\")\n}\n";
        assert_eq!(
            routes_of("Api.java", java),
            ["GET /api/items", "POST /api/items"]
        );
    }

    #[test]
    fn finds_rust_php_ruby_and_django_routes() {
        let rs = "#[get(\"/health\")]\nlet app = Router::new().route(\"/users\", get(list).post(create));\n";
        assert_eq!(routes_of("main.rs", rs), ["GET /health", "GET,POST /users"]);

        let php = "Route::get('/home', [C::class, 'i']);\nRoute::resource('photos', P::class);\n";
        assert_eq!(routes_of("web.php", php), ["GET /home", "CRUD /photos"]);

        let rb = "Rails.application.routes.draw do\n  root \"home#index\"\n  get \"/about\", to: \"p#a\"\n  resources :users\nend\n";
        assert_eq!(
            routes_of("config/routes.rb", rb),
            ["GET /", "GET /about", "CRUD /users"]
        );
        assert!(routes_of("app/models/user.rb", "get 'x'\n").is_empty());

        let dj = "urlpatterns = [path('users/', v), re_path(r'^api/$', v)]\n";
        assert_eq!(routes_of("app/urls.py", dj), ["ANY /users", "ANY /api"]);
    }

    #[test]
    fn finds_next_routes() {
        let route = "export async function GET(req) {}\nexport const POST = h;\n";
        assert_eq!(
            routes_of("src/app/api/(admin)/users/[id]/route.ts", route),
            ["GET /api/users/[id]", "POST /api/users/[id]"]
        );
        assert_eq!(
            routes_of("pages/api/hello.ts", "export default h"),
            ["ANY /api/hello"]
        );
        assert_eq!(routes_of("pages/api/index.js", ""), ["ANY /api"]);
    }

    #[test]
    fn runbook_reads_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "package.json",
            r#"{"scripts": {"start": "node server.js"}}"#,
        );
        write(root, ".env", "LEAKED_NAME=super-secret-value\n");
        write(root, ".env.example", "PORT=3000\n");
        write(root, "server.js", "const port = process.env.PORT;\nconst k = process.env.API_KEY;\napp.get('/health', h);\n");

        let book = runbook(root);
        assert_eq!(book.commands[0].run, "npm run start");
        let names: Vec<&str> = book.env_vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["API_KEY", "PORT"]);
        assert!(book.env_vars[1].documented);
        assert!(!book.env_vars[0].documented);
        assert_eq!(book.routes[0].path, "/health");

        let json = serde_json::to_string(&book).unwrap();
        let text = render_text(&book);
        for output in [&json, &text] {
            assert!(!output.contains("LEAKED_NAME"));
            assert!(!output.contains("super-secret-value"));
            assert!(!output.contains("3000"));
        }
        assert!(text.contains("npm run start"));
        assert!(text.contains("not in an example file"));
    }

    #[test]
    fn output_is_sanitized_and_capped() {
        let long = "x".repeat(500);
        let cmds = make_targets("Makefile", &format!("{long}:\n"));
        assert!(cmds[0].run.chars().count() <= MAX_TEXT + 1);
        let routes = routes_of("a.ts", "app.get('/a\u{1b}[2J', h)\n");
        assert_eq!(routes, ["GET /a?[2J"]);
    }

    #[test]
    fn empty_project_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let book = runbook(dir.path());
        assert!(render_text(&book).contains("No scripts"));
    }
}
