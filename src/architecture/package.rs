//! Package purpose: what each package in a repository is for, read from
//! its manifest, before any file is given a layer.
//!
//! A published library's `routes/` folder or a CLI's project templates are
//! not the layers of an application, so files in library, CLI and template
//! packages are kept out of the layers (see `roles::place`). When the
//! evidence is weak the purpose is `unknown` and files are placed as before:
//! a wrong `library` hides real layers, a wrong `unknown` changes nothing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// What a package is for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    /// A deployable program: web app, service, desktop or mobile app.
    Application,
    /// A command-line tool installed from a registry.
    Cli,
    /// A package published for other code to import.
    Library,
    /// A project template (`cookiecutter.json`).
    Template,
    /// Not enough evidence, or a workspace root.
    Unknown,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Application => "application",
            Purpose::Cli => "cli",
            Purpose::Library => "library",
            Purpose::Template => "template",
            Purpose::Unknown => "unknown",
        }
    }
}

/// One package: its folder relative to the repository root and why it has
/// its purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub dir: PathBuf,
    pub purpose: Purpose,
    pub signals: Vec<String>,
}

impl Package {
    /// `.` for the root, else the folder with forward slashes.
    pub fn display(&self) -> String {
        if self.dir.as_os_str().is_empty() {
            ".".to_string()
        } else {
            self.dir.to_string_lossy().replace('\\', "/")
        }
    }
}

/// Where a file sits for layer purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Product code: counts as layers.
    Product,
    /// A test harness, mock, story, benchmark or codemod: never a layer.
    Harness,
    /// Inside a library or CLI package (index into the package list).
    NotAnApp(usize),
    /// Files a template generates: layers only when nothing else has any.
    Template,
    /// An example or docs app: layers only when nothing else has any.
    Sample,
}

/// Folders that hold test harnesses and tooling rather than product code.
pub const HARNESS_DIRS: &[&str] = &[
    "mocks",
    "mock",
    "__mocks__",
    "testing",
    "test-utils",
    "test_utils",
    "testutils",
    "fixtures",
    "__fixtures__",
    "playwright",
    "cypress",
    "_examples",
    "benchmarks",
    "benchmark",
    "bench",
    "codemods",
    "codemod",
    ".storybook",
    "playground",
    "playgrounds",
];

/// Frameworks and ORMs whose presence in `dependencies` makes a JavaScript
/// package an application (or at least not a plain library).
const JS_APP_DEPS: &[&str] = &[
    "express",
    "koa",
    "fastify",
    "@nestjs/core",
    "@hapi/hapi",
    "next",
    "nuxt",
    "@sveltejs/kit",
    "gatsby",
    "astro",
    "react-scripts",
    "react",
    "react-dom",
    "vue",
    "svelte",
    "solid-js",
    "@angular/core",
    "@prisma/client",
    "typeorm",
    "sequelize",
    "mongoose",
    "drizzle-orm",
    "knex",
    "electron",
    "@trpc/server",
    "@koa/router",
];

/// Python web frameworks and ORMs.
const PY_APP_DEPS: &[&str] = &[
    "django",
    "fastapi",
    "flask",
    "starlette",
    "sqlalchemy",
    "sqlmodel",
    "tornado",
    "sanic",
    "pyramid",
    "litestar",
    "wagtail",
];

/// Files that only a deployed Python program has.
const PY_APP_FILES: &[&str] = &["manage.py", "alembic.ini", "wsgi.py", "asgi.py", "Procfile"];

/// Go modules that make a module a web application.
const GO_APP_DEPS: &[&str] = &[
    "github.com/gin-gonic/gin",
    "github.com/labstack/echo",
    "github.com/go-chi/chi",
    "github.com/gofiber/fiber",
    "gorm.io/gorm",
    "github.com/gorilla/mux",
];

/// Rust crates that make a crate a web application.
const RUST_APP_DEPS: &[&str] = &[
    "axum",
    "actix-web",
    "rocket",
    "warp",
    "diesel",
    "sqlx",
    "sea-orm",
];

/// The signal of a crate whose targets are read from its files.
const RUST_UNDECIDED: &str = "Cargo.toml: [package]";

/// Largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Every package in a repository, deepest folder first for lookups.
#[derive(Debug, Default)]
pub struct Packages {
    pub list: Vec<Package>,
}

/// Folder names that mark a published package as a client SDK or shared
/// type models rather than shared product code.
const SDK_NAMES: &[&str] = &["sdk", "client", "types", "typings"];

/// Whether a package folder is named like a client SDK or type models
/// (`sdk`, `api-client`, `turbo-types`).
fn is_sdk_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.split(['-', '_', '.'])
                .any(|part| SDK_NAMES.contains(&part))
        })
}

/// Whether a relative path has a folder in `names`.
fn has_dir(path: &Path, names: &[&str]) -> bool {
    path.parent().is_some_and(|dir| {
        dir.iter()
            .filter_map(|c| c.to_str())
            .any(|c| names.contains(&c.to_ascii_lowercase().as_str()))
    })
}

/// Whether a relative path is in a harness folder or is a story.
pub fn is_harness(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains(".stories.") || name.contains(".story.") || has_dir(path, HARNESS_DIRS)
}

/// Whether a path sits under a templated folder (`{{cookiecutter.x}}`).
fn is_templated(path: &Path) -> bool {
    path.iter()
        .filter_map(|c| c.to_str())
        .any(|c| c.contains("{{"))
}

/// Whether a folder name holds project templates (`template`,
/// `templates`, `template-react`).
fn is_template_dir(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "templates" || name == "template" || name.starts_with("template-")
}

fn read(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

impl Packages {
    /// Find packages from the manifests among `files`.
    pub fn detect(root: &Path, files: &[PathBuf]) -> Self {
        let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
        let relative: Vec<PathBuf> = files.iter().map(|f| rel(f)).collect();
        let mut by_dir: BTreeMap<PathBuf, Vec<(Purpose, String)>> = BTreeMap::new();
        let mut spa: BTreeMap<PathBuf, String> = BTreeMap::new();
        for (file, relative) in files.iter().zip(&relative) {
            let Some(name) = relative.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let dir = relative.parent().map(Path::to_path_buf).unwrap_or_default();
            // Manifests of tests, samples, harnesses and generated projects
            // describe something else.
            if crate::architecture::is_test(relative)
                || crate::framework_detector::in_sample_dir(relative)
                || is_harness(relative)
                || is_templated(relative)
            {
                continue;
            }
            let found = match name {
                "package.json" => read(file).map(|t| {
                    if let Some(framework) = js_spa(&t) {
                        spa.insert(dir.clone(), framework);
                    }
                    js_purpose(&t)
                }),
                "pyproject.toml" | "setup.py" | "setup.cfg" => {
                    read(file).map(|t| python_purpose(name, &t))
                }
                "go.mod" => read(file).map(|t| go_purpose(root, &t, &dir, files)),
                "Cargo.toml" => read(file).map(|t| rust_purpose(&t)),
                "Gemfile" => read(file).map(|t| ruby_purpose(None, &t)),
                "composer.json" => read(file).map(|t| php_purpose(&t)),
                "mix.exs" | "pom.xml" | "build.gradle" | "build.gradle.kts" => {
                    Some((Purpose::Unknown, format!("{name} (purpose not read)")))
                }
                "cookiecutter.json" => Some((
                    Purpose::Template,
                    "cookiecutter.json: a project template".to_string(),
                )),
                "manage.py" => Some((Purpose::Application, "manage.py".to_string())),
                _ if name.ends_with(".gemspec") => Some(ruby_purpose(Some(name), "")),
                _ => None,
            };
            if let Some(found) = found {
                by_dir.entry(dir).or_default().push(found);
            }
        }

        // What the files beside a manifest add: a Rust binary or library
        // target, a Python program's entry files.
        let python: Vec<PathBuf> = by_dir
            .iter()
            .filter(|(_, found)| {
                found
                    .iter()
                    .any(|(_, s)| s.starts_with("pyproject.toml") || s.starts_with("setup."))
            })
            .map(|(dir, _)| dir.clone())
            .collect();
        let rust: Vec<PathBuf> = by_dir
            .iter()
            .filter(|(_, found)| found.iter().any(|(_, s)| s == RUST_UNDECIDED))
            .map(|(dir, _)| dir.clone())
            .collect();
        let mut rust_targets: BTreeMap<PathBuf, (bool, bool)> = BTreeMap::new();
        for (file, relative) in files.iter().zip(&relative) {
            if is_harness(relative) || crate::architecture::is_test(relative) {
                continue;
            }
            // A browser app bundled at build time keeps its framework in
            // `devDependencies`; its `index.html` shows it is an app.
            if relative.file_name().is_some_and(|n| n == "index.html") {
                let parent = relative.parent().unwrap_or(Path::new(""));
                for (dir, framework) in &spa {
                    if parent == dir || parent == dir.join("public") || parent == dir.join("src") {
                        by_dir.entry(dir.clone()).or_default().push((
                            Purpose::Application,
                            format!(
                                "package.json: {framework} app with index.html and a dev server"
                            ),
                        ));
                    }
                }
            }
            for dir in &python {
                if let Some(name) = relative
                    .strip_prefix(dir)
                    .ok()
                    .and_then(|below| python_app_file(below, file))
                {
                    by_dir
                        .entry(dir.clone())
                        .or_default()
                        .push((Purpose::Application, name));
                }
            }
            for dir in &rust {
                let Ok(below) = relative.strip_prefix(dir) else {
                    continue;
                };
                let s = below.to_string_lossy().replace('\\', "/");
                let entry = rust_targets.entry(dir.clone()).or_default();
                if s == "src/main.rs" || (s.starts_with("src/bin/") && s.ends_with(".rs")) {
                    entry.0 = true;
                } else if s == "src/lib.rs" {
                    entry.1 = true;
                }
            }
        }
        for (dir, (bin, lib)) in rust_targets {
            let found = by_dir.entry(dir).or_default();
            if bin {
                found.push((
                    Purpose::Application,
                    "Cargo.toml with src/main.rs".to_string(),
                ));
            } else if lib {
                found.push((
                    Purpose::Library,
                    "Cargo.toml with src/lib.rs, no binary or web framework".to_string(),
                ));
            }
        }

        let mut list: Vec<Package> = by_dir
            .into_iter()
            .map(|(dir, found)| {
                // Template beats everything; any application signal beats
                // library and CLI ones (a framework in the dependencies, a
                // `manage.py` beside `pyproject.toml`).
                let purpose = found
                    .iter()
                    .map(|(p, _)| *p)
                    .min_by_key(|p| match p {
                        Purpose::Template => 0,
                        Purpose::Application => 1,
                        Purpose::Cli => 2,
                        Purpose::Library => 3,
                        Purpose::Unknown => 4,
                    })
                    .unwrap_or(Purpose::Unknown);
                let mut signals: Vec<String> = found
                    .into_iter()
                    .filter(|(p, _)| *p == purpose)
                    .map(|(_, s)| s)
                    .collect();
                signals.dedup();
                Package {
                    dir,
                    purpose,
                    signals,
                }
            })
            .collect();

        // Packages inside a CLI's template folders are the projects it
        // generates, not packages of this repository.
        let clis: Vec<PathBuf> = list
            .iter()
            .filter(|p| p.purpose == Purpose::Cli)
            .map(|p| p.dir.clone())
            .collect();
        list.retain(|p| {
            !clis
                .iter()
                .any(|cli| p.dir != *cli && template_below(cli, &p.dir))
        });
        list.sort_by(|a, b| {
            b.dir
                .components()
                .count()
                .cmp(&a.dir.components().count())
                .then_with(|| a.dir.cmp(&b.dir))
        });
        Packages { list }
    }

    /// Whether the root manifest makes the whole repository a library or
    /// CLI; applications nested in it are then demos or test apps.
    fn root_is_library(&self) -> bool {
        self.list.iter().any(|p| {
            p.dir.as_os_str().is_empty() && matches!(p.purpose, Purpose::Library | Purpose::Cli)
        })
    }

    /// Whether the repository has an application its libraries serve.
    fn serves_application(&self) -> bool {
        !self.root_is_library() && self.list.iter().any(|p| p.purpose == Purpose::Application)
    }

    /// Whether a package's own files stay out of application layers: every
    /// CLI, and libraries unless the repository also has an application
    /// they serve (then only SDK and type-model packages stay out).
    pub fn excluded(&self, index: usize) -> bool {
        let p = &self.list[index];
        match p.purpose {
            Purpose::Cli => true,
            Purpose::Library => !self.serves_application() || is_sdk_dir(&p.dir),
            _ => false,
        }
    }

    /// Index of the package `path` belongs to: the closest folder with a
    /// manifest.
    pub fn of(&self, path: &Path) -> Option<usize> {
        self.list
            .iter()
            .position(|p| p.dir.as_os_str().is_empty() || path.starts_with(&p.dir))
    }

    /// Where a relative path sits for layer purposes.
    pub fn scope(&self, path: &Path) -> Scope {
        if is_harness(path) {
            return Scope::Harness;
        }
        let package = self.of(path);
        if let Some(index) = package {
            let p = &self.list[index];
            match p.purpose {
                Purpose::Template => {
                    if is_templated(path) {
                        return Scope::Template;
                    }
                    // Cookiecutter hooks run when the template is used.
                    let below = path.strip_prefix(&p.dir).unwrap_or(path);
                    if below
                        .components()
                        .next()
                        .is_some_and(|c| c.as_os_str() == "hooks")
                    {
                        return Scope::Harness;
                    }
                }
                Purpose::Cli if template_below(&p.dir, path) => return Scope::Template,
                Purpose::Application if !p.dir.as_os_str().is_empty() && self.root_is_library() => {
                    return Scope::Sample;
                }
                Purpose::Library | Purpose::Cli if self.excluded(index) => {
                    return Scope::NotAnApp(index);
                }
                _ => {}
            }
        }
        if is_templated(path) {
            return Scope::Template;
        }
        if crate::framework_detector::in_sample_dir(path) {
            return Scope::Sample;
        }
        Scope::Product
    }

    /// Whether any package is a library or CLI.
    pub fn has_non_app(&self) -> bool {
        (0..self.list.len()).any(|i| self.excluded(i))
    }
}

/// Source languages whose files are not read for layers.
const UNREAD_LANGUAGES: &[(&str, &[&str])] = &[
    ("Elixir", &["ex", "exs"]),
    ("Erlang", &["erl"]),
    ("Scala", &["scala"]),
    ("Clojure", &["clj", "cljs"]),
    ("Swift", &["swift"]),
    ("Dart", &["dart"]),
    ("C", &["c", "h"]),
    ("C++", &["cpp", "cc", "hpp", "cxx"]),
    ("Lua", &["lua"]),
    ("Haskell", &["hs"]),
    ("OCaml", &["ml"]),
    ("R", &["r"]),
    ("Julia", &["jl"]),
    ("Zig", &["zig"]),
    ("Nim", &["nim"]),
    ("Crystal", &["cr"]),
    ("Elm", &["elm"]),
    ("F#", &["fs"]),
    ("Groovy", &["groovy"]),
];

/// Fewest unread files that can make the architecture unknown.
const MIN_UNREAD: usize = 20;

/// When more product source files are in unread languages than in read
/// ones (and at least [`MIN_UNREAD`]): the main unread language, its file
/// count, and the count of read files. Tests, samples and harnesses are
/// left out of both counts.
pub fn unread_majority(root: &Path, files: &[PathBuf]) -> Option<(String, usize, usize)> {
    let mut read = 0usize;
    let mut unread: BTreeMap<&str, usize> = BTreeMap::new();
    for file in files {
        let relative = file.strip_prefix(root).unwrap_or(file);
        let Some(ext) = relative.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        let supported = ext == "astro" || crate::runbook::CODE_EXTENSIONS.contains(&ext.as_str());
        let language = UNREAD_LANGUAGES
            .iter()
            .find(|(_, exts)| exts.contains(&ext.as_str()))
            .map(|(name, _)| *name);
        if !supported && language.is_none() {
            continue;
        }
        if crate::architecture::is_test(relative)
            || crate::framework_detector::in_sample_dir(relative)
            || is_harness(relative)
        {
            continue;
        }
        match language {
            Some(name) if !supported => *unread.entry(name).or_default() += 1,
            _ => read += 1,
        }
    }
    let total: usize = unread.values().sum();
    if total < MIN_UNREAD || total <= read {
        return None;
    }
    let (name, _) = unread
        .iter()
        .max_by_key(|(name, n)| (**n, std::cmp::Reverse(**name)))?;
    let language = if unread.len() == 1 {
        name.to_string()
    } else {
        format!("mostly {name}")
    };
    Some((language, total, read))
}

/// Whether `path` is inside a template folder below `dir`.
fn template_below(dir: &Path, path: &Path) -> bool {
    let Ok(below) = path.strip_prefix(dir) else {
        return false;
    };
    let mut parts: Vec<&str> = below
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    // The last part of a file path is the file itself.
    if path.extension().is_some() {
        parts.pop();
    }
    parts.iter().any(|p| is_template_dir(p))
}

// ---------------------------------------------------------------------------
// Per-ecosystem purpose
// ---------------------------------------------------------------------------

/// A JavaScript package's purpose from its `package.json`.
fn js_purpose(text: &str) -> (Purpose, String) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return (Purpose::Unknown, "package.json not readable".to_string());
    };
    let private = json.get("private").and_then(|v| v.as_bool()) == Some(true);
    let publish: Vec<&str> = ["exports", "module", "types", "typings", "files"]
        .into_iter()
        .filter(|k| json.get(*k).is_some())
        .collect();
    let bin = json.get("bin").is_some();
    // Registry dependencies only: `workspace:` and `file:` ones are this
    // repository's own packages.
    let framework: Option<String> = json
        .get("dependencies")
        .and_then(|d| d.as_object())
        .and_then(|deps| {
            deps.iter()
                .filter(|(_, v)| {
                    v.as_str().is_none_or(|v| {
                        !(v.starts_with("workspace:")
                            || v.starts_with("file:")
                            || v.starts_with("link:"))
                    })
                })
                .map(|(k, _)| k.as_str())
                .find(|k| JS_APP_DEPS.contains(k) || k.starts_with("@remix-run/"))
                .map(str::to_string)
        });
    if let Some(name) = framework {
        return (
            Purpose::Application,
            format!("package.json depends on {name}"),
        );
    }
    if !private && (bin || !publish.is_empty()) {
        let what = if publish.is_empty() {
            "bin".to_string()
        } else {
            publish.join(", ")
        };
        return if bin {
            (
                Purpose::Cli,
                format!("package.json: published with bin ({what}), no app framework"),
            )
        } else {
            (
                Purpose::Library,
                format!("package.json: published ({what}), no app framework"),
            )
        };
    }
    if json.get("workspaces").is_some() {
        return (Purpose::Unknown, "package.json: workspace root".to_string());
    }
    (
        Purpose::Unknown,
        "package.json: no publish or app signals".to_string(),
    )
}

/// Browser frameworks and dev servers that, with an `index.html`, make a
/// package a bundled browser app.
const JS_SPA_FRAMEWORKS: &[&str] = &[
    "react-dom",
    "vue",
    "svelte",
    "solid-js",
    "preact",
    "@angular/core",
];
const JS_DEV_SERVERS: &[&str] = &[
    "vite",
    "webpack",
    "parcel",
    "rsbuild",
    "react-scripts",
    "vue-cli-service",
    "ng",
];

/// The browser framework of a package whose scripts run a dev server, in
/// any dependency list.
fn js_spa(text: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let scripts = json.get("scripts")?.as_object()?;
    let serves = scripts.values().filter_map(|v| v.as_str()).any(|script| {
        let words: Vec<&str> = script.split([' ', '&', ';']).collect();
        JS_DEV_SERVERS.iter().any(|tool| words.contains(tool))
    });
    if !serves {
        return None;
    }
    ["dependencies", "devDependencies"]
        .into_iter()
        .filter_map(|k| json.get(k).and_then(|d| d.as_object()))
        .flat_map(|deps| deps.keys())
        .find(|k| JS_SPA_FRAMEWORKS.contains(&k.as_str()))
        .map(|k| k.to_string())
}

/// Package names in a Python requirement list (`"fastapi>=0.100"`).
fn requirement_name(spec: &str) -> String {
    spec.trim()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect::<String>()
        .to_ascii_lowercase()
        .replace('_', "-")
}

/// A Python package's purpose from `pyproject.toml`, `setup.py` or
/// `setup.cfg`, and the files beside it.
fn python_purpose(name: &str, text: &str) -> (Purpose, String) {
    let mut deps: Vec<String> = Vec::new();
    let mut scripts = false;
    let mut named = false;
    match name {
        "pyproject.toml" => {
            let Ok(table) = text.parse::<toml::Table>() else {
                return (Purpose::Unknown, "pyproject.toml not readable".to_string());
            };
            if let Some(project) = table.get("project").and_then(|p| p.as_table()) {
                named = project.contains_key("name");
                scripts = project.contains_key("scripts") || project.contains_key("gui-scripts");
                if let Some(list) = project.get("dependencies").and_then(|d| d.as_array()) {
                    deps.extend(list.iter().filter_map(|d| d.as_str()).map(requirement_name));
                }
            }
            if let Some(poetry) = table
                .get("tool")
                .and_then(|t| t.get("poetry"))
                .and_then(|p| p.as_table())
            {
                named |= poetry.contains_key("name");
                scripts |= poetry.contains_key("scripts");
                if let Some(d) = poetry.get("dependencies").and_then(|d| d.as_table()) {
                    deps.extend(d.keys().map(|k| requirement_name(k)));
                }
            }
        }
        _ => {
            named = true;
            scripts = text.contains("console_scripts") || text.contains("entry_points");
            let lower = text.to_ascii_lowercase();
            deps.extend(
                PY_APP_DEPS
                    .iter()
                    .filter(|d| {
                        lower.contains(&format!("\"{d}")) || lower.contains(&format!("'{d}"))
                    })
                    .map(|d| d.to_string()),
            );
            // `install_requires = django>=4` in setup.cfg.
            if name == "setup.cfg" {
                deps.extend(lower.lines().map(requirement_name));
            }
        }
    }
    if let Some(dep) = deps.iter().find(|d| PY_APP_DEPS.contains(&d.as_str())) {
        return (Purpose::Application, format!("{name} depends on {dep}"));
    }
    if scripts {
        return (Purpose::Cli, format!("{name}: declares console scripts"));
    }
    if named {
        return (
            Purpose::Library,
            format!("{name}: a named package, no web framework"),
        );
    }
    (Purpose::Unknown, format!("{name}: tool settings only"))
}

/// Strong signs of a deployed Python program next to a manifest, checked
/// once files are known (`manage.py`, `alembic.ini`, `app/main.py`).
fn python_app_file(below: &Path, file: &Path) -> Option<String> {
    let name = below.file_name()?.to_str()?;
    let depth = below.components().count();
    if PY_APP_FILES.contains(&name) && depth <= 3 {
        // A library's `wsgi.py` or `asgi.py` is an adapter module; a
        // program's defines the `application` (or `app`) a server runs.
        if matches!(name, "wsgi.py" | "asgi.py") && !defines_server_app(file) {
            return None;
        }
        return Some(name.to_string());
    }
    let s = below.to_string_lossy().replace('\\', "/");
    if s == "app/main.py" {
        return Some(s);
    }
    None
}

/// Whether a Python file assigns a top-level `application` or `app`.
fn defines_server_app(file: &Path) -> bool {
    read(file).is_some_and(|text| {
        text.lines().any(|line| {
            ["application", "app"].iter().any(|name| {
                line.strip_prefix(name)
                    .is_some_and(|rest| rest.trim_start().starts_with('=') || rest.starts_with(':'))
            })
        })
    })
}

/// A Go module's purpose: a module with a `package main` outside tests,
/// samples and harnesses, or one that requires a web framework, is an
/// application; otherwise it is a library.
fn go_purpose(root: &Path, text: &str, dir: &Path, files: &[PathBuf]) -> (Purpose, String) {
    if let Some(dep) = text
        .lines()
        .map(str::trim)
        .map(|l| l.trim_start_matches("require").trim())
        .find_map(|l| GO_APP_DEPS.iter().find(|d| l.starts_with(**d)))
    {
        return (Purpose::Application, format!("go.mod requires {dep}"));
    }
    let main = files.iter().find(|f| {
        let relative = f.strip_prefix(root).unwrap_or(f);
        relative.starts_with(dir)
            && relative.extension().is_some_and(|e| e == "go")
            && !crate::architecture::is_test(relative)
            && !crate::framework_detector::in_sample_dir(relative)
            && !is_harness(relative)
            && read(f).is_some_and(|t| {
                t.lines()
                    .map(str::trim)
                    .any(|l| l == "package main" || l.starts_with("package main "))
            })
    });
    match main {
        Some(f) => (
            Purpose::Application,
            format!(
                "package main in {}",
                f.strip_prefix(root)
                    .unwrap_or(f)
                    .to_string_lossy()
                    .replace('\\', "/")
            ),
        ),
        None => (
            Purpose::Library,
            "go.mod: no package main outside examples and tests".to_string(),
        ),
    }
}

/// A Rust crate's purpose from `Cargo.toml` and its source layout.
fn rust_purpose(text: &str) -> (Purpose, String) {
    let Ok(table) = text.parse::<toml::Table>() else {
        return (Purpose::Unknown, "Cargo.toml not readable".to_string());
    };
    if !table.contains_key("package") {
        return (Purpose::Unknown, "Cargo.toml: workspace root".to_string());
    }
    let framework = table
        .get("dependencies")
        .and_then(|d| d.as_table())
        .and_then(|deps| {
            deps.iter()
                .filter(|(_, v)| v.as_table().is_none_or(|t| !t.contains_key("path")))
                .map(|(k, _)| k.as_str())
                .find(|k| RUST_APP_DEPS.contains(k))
                .map(str::to_string)
        });
    let bin = table.contains_key("bin");
    let lib = table.contains_key("lib");
    if let Some(dep) = framework {
        return (Purpose::Application, format!("Cargo.toml depends on {dep}"));
    }
    if bin {
        return (Purpose::Application, "Cargo.toml: [[bin]]".to_string());
    }
    if lib {
        return (Purpose::Library, "Cargo.toml: [lib], no binary".to_string());
    }
    // Decided from src/main.rs and src/lib.rs once files are known.
    (Purpose::Unknown, RUST_UNDECIDED.to_string())
}

/// A Ruby project's purpose from its gemspec or Gemfile.
fn ruby_purpose(gemspec: Option<&str>, text: &str) -> (Purpose, String) {
    if let Some(name) = gemspec {
        return (Purpose::Library, format!("{name}: a published gem"));
    }
    let rails = text
        .lines()
        .map(str::trim)
        .any(|l| l.starts_with("gem 'rails'") || l.starts_with("gem \"rails\""));
    if rails {
        (Purpose::Application, "Gemfile depends on rails".to_string())
    } else {
        (Purpose::Unknown, "Gemfile".to_string())
    }
}

/// A PHP project's purpose from `composer.json`.
fn php_purpose(text: &str) -> (Purpose, String) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return (Purpose::Unknown, "composer.json not readable".to_string());
    };
    let requires_laravel = json
        .get("require")
        .and_then(|r| r.as_object())
        .is_some_and(|r| {
            r.contains_key("laravel/framework") || r.contains_key("symfony/framework-bundle")
        });
    match json.get("type").and_then(|t| t.as_str()) {
        Some("project") => (
            Purpose::Application,
            "composer.json type project".to_string(),
        ),
        _ if requires_laravel => (
            Purpose::Application,
            "composer.json requires a web framework".to_string(),
        ),
        Some("library") => (Purpose::Library, "composer.json type library".to_string()),
        _ => (Purpose::Unknown, "composer.json".to_string()),
    }
}
