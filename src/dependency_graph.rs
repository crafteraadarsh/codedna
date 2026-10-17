//! Dependency graph module — parallel build via `rayon`.
//!
//! Parses import/require statements in `.ts`, `.js`, `.py`, and `.rs` files,
//! then builds a directed graph:
//!
//! ```text
//! src/server.ts     → [src/api/routes.ts]
//! src/api/routes.ts → [src/controllers/user.ts]
//! ```
//!
//! Only relative/internal imports are tracked. Third-party packages and
//! stdlib imports are ignored.

use crate::cache::Cache;
use crate::facts::{Certainty, DependencyFact, DependencyKind, Resolution};
use crate::graph;
use rayon::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A directed dependency graph.
///
/// - key   : absolute path of the source file
/// - value : absolute paths of files it imports/requires
pub type DependencyGraph = HashMap<PathBuf, Vec<PathBuf>>;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a dependency graph from a list of repository files.
///
/// For each file whose extension is `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`,
/// `.py`, or `.rs`, the file is parsed and its relative imports are resolved
/// to absolute paths that exist on disk.
///
/// Only those source files become nodes: READMEs, configs, lockfiles and
/// other files that cannot import anything are left out, so they are never
/// reported as dead code. Files that cannot be read are kept with no edges.
#[cfg(test)]
pub fn build_dependency_graph(files: &[PathBuf]) -> DependencyGraph {
    build_with_tallies(files).0
}

/// What happened to the imports of one source file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportTally {
    /// Whether the file could be read at all.
    pub readable: bool,
    /// Imports of the repository's own files that were found on disk.
    pub resolved: usize,
    /// Imports that point into the repository but matched no file, as
    /// `(line, import)`: relative paths, configured aliases, workspace
    /// packages and local Python packages.
    pub unresolved: Vec<(usize, String)>,
    /// Imports of packages from outside the repository (npm packages, the
    /// standard library, ...).
    pub external: usize,
}

impl ImportTally {
    fn internal(&mut self, line: usize, import: &str, found: bool) {
        if found {
            self.resolved += 1;
        } else {
            self.unresolved.push((line, import.to_string()));
        }
    }
}

/// Build the dependency graph and record, for every source file, how many
/// of its imports were resolved, left unresolved or point outside the
/// repository.
pub fn build_with_tallies(files: &[PathBuf]) -> (DependencyGraph, HashMap<PathBuf, ImportTally>) {
    build_cached(files, &Cache::off())
}

/// [`build_with_tallies`], reusing the imports `cache` holds for files
/// that have not changed.
pub fn build_cached(
    files: &[PathBuf],
    cache: &Cache,
) -> (DependencyGraph, HashMap<PathBuf, ImportTally>) {
    let (graph, tallies, _) = build_facts(files, cache);
    (graph, tallies)
}

/// Extract the dependency facts of every source file, then build the graph
/// from them. Facts come back in the order they were found, which is the
/// order of each file's edges; [`crate::facts::sort`] puts them in report
/// order.
pub fn build_facts(
    files: &[PathBuf],
    cache: &Cache,
) -> (
    DependencyGraph,
    HashMap<PathBuf, ImportTally>,
    Vec<DependencyFact>,
) {
    BUILD.fetch_add(1, Ordering::Relaxed);
    let aliases = Aliases::from_files(files);
    let python_packages = python_packages(files);

    // Each file is parsed independently — no shared mutable state needed.
    let parsed: Vec<(PathBuf, Vec<DependencyFact>, ImportTally)> = files
        .par_iter()
        .filter(|file| is_source_file(file))
        .map(|file| {
            let key = normalize(file);
            let (facts, tally) = cache.imports(file, || {
                let mut tally = ImportTally::default();
                let facts = parse_dependencies_with(&key, &aliases, &python_packages, &mut tally);
                (facts, tally)
            });
            (key, facts, tally)
        })
        .collect();

    let mut nodes = Vec::with_capacity(parsed.len());
    let mut facts = Vec::new();
    let mut tallies = HashMap::with_capacity(parsed.len());
    for (key, file_facts, tally) in parsed {
        tallies.insert(key.clone(), tally);
        nodes.push(key);
        facts.extend(file_facts);
    }
    facts.extend(django_app_facts(files));
    let graph = graph::from_facts(nodes, &facts);
    (graph, tallies, facts)
}

/// Modules Django imports from every installed app without an import
/// statement.
const DJANGO_APP_MODULES: &[&str] = &[
    "models",
    "admin",
    "apps",
    "signals",
    "receivers",
    "checks",
    "translation",
    "tasks",
];

/// Whether `dir` looks like a Django app: a package with an `apps.py`, a
/// `models.py` or a `migrations/` package. `exists` says whether a file is
/// there.
fn is_django_app(dir: &Path, exists: impl Fn(&Path) -> bool) -> bool {
    exists(&dir.join("__init__.py"))
        && ["apps.py", "models.py", "migrations/__init__.py"]
            .iter()
            .any(|f| exists(&dir.join(f)))
}

/// Folders inside a Django app that Django loads by name.
const DJANGO_APP_DIRS: &[&str] = &["templatetags", "management", "migrations"];

/// Facts linking each Django app's `__init__.py` and `apps.py` to the modules Django
/// loads from it by convention, so an installed app keeps them reachable.
/// An app is a package whose folder holds an `apps.py`, a `models.py` or
/// `migrations/`.
fn django_app_facts(files: &[PathBuf]) -> Vec<DependencyFact> {
    let keys: HashSet<PathBuf> = files.iter().map(|f| normalize(f)).collect();
    let apps: HashSet<PathBuf> = keys
        .iter()
        .filter_map(|f| f.parent())
        .filter(|dir| is_django_app(dir, |p| keys.contains(p)))
        .map(Path::to_path_buf)
        .collect();
    if apps.is_empty() {
        return Vec::new();
    }

    let mut loaded: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for file in files.iter().map(|f| normalize(f)) {
        if file.extension().is_none_or(|e| e != "py") {
            continue;
        }
        let Some(parent) = file.parent() else {
            continue;
        };
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        // `app/models.py`, or `app/models/__init__.py`.
        let direct = apps.contains(parent) && DJANGO_APP_MODULES.contains(&stem);
        let package = stem == "__init__"
            && parent
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| DJANGO_APP_MODULES.contains(&n))
            && parent.parent().is_some_and(|app| apps.contains(app));
        // Anything under `app/templatetags/`, `app/management/`, `app/migrations/`.
        let in_dir = parent.ancestors().any(|dir| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| DJANGO_APP_DIRS.contains(&n))
                && dir.parent().is_some_and(|app| apps.contains(app))
        });
        if !(direct || package || in_dir) {
            continue;
        }
        let app = parent
            .ancestors()
            .find(|dir| apps.contains(*dir))
            .map(Path::to_path_buf);
        if let Some(app) = app {
            loaded.entry(app).or_default().push(file);
        }
    }

    let mut facts = Vec::new();
    for (app, modules) in loaded {
        // Only files that exist: an app need not have an `apps.py`.
        for from in [app.join("__init__.py"), app.join("apps.py")] {
            if !keys.contains(&from) {
                continue;
            }
            for module in modules.iter().filter(|m| **m != from) {
                let name: Vec<String> = module
                    .strip_prefix(&app)
                    .unwrap_or(module)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                facts.push(
                    DependencyFact::new(
                        &from,
                        &name.join("/"),
                        DependencyKind::ConfiguredEntry,
                        Certainty::Inferred,
                        "python.django_app_module",
                    )
                    .resolved(vec![module.clone()]),
                );
            }
        }
    }
    facts
}

/// Whether `ext` (lowercase) is a source extension whose imports CodeDna
/// follows.
pub fn follows_imports(ext: &str) -> bool {
    SOURCE_EXTENSIONS.contains(&ext)
}

/// Names of the repository's own Python packages: directories holding an
/// `__init__.py`. An absolute import of one of these that matches no file
/// is unresolved rather than a third-party package.
fn python_packages(files: &[PathBuf]) -> HashSet<String> {
    files
        .iter()
        .filter(|f| f.file_name().is_some_and(|n| n == "__init__.py"))
        .filter_map(|f| f.parent()?.file_name()?.to_str().map(str::to_string))
        .collect()
}

/// Counts graph builds, so the per-thread file cache below starts empty for
/// each build and never answers from an older scan.
static BUILD: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Whether a path is a file (`Some(true)`), a directory (`Some(false)`)
    /// or neither (`None`), for the build numbered in the first field.
    /// Import resolution asks about the same paths over and over.
    static KINDS: RefCell<(u64, HashMap<PathBuf, Option<bool>>)> = RefCell::new((0, HashMap::new()));
}

/// What `path` is on disk, cached for the current build.
fn path_kind(path: &Path) -> Option<bool> {
    let build = BUILD.load(Ordering::Relaxed);
    KINDS.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.0 != build {
            *cache = (build, HashMap::new());
        }
        if let Some(&kind) = cache.1.get(path) {
            return kind;
        }
        let kind = fs::metadata(path).ok().and_then(|m| {
            if m.is_file() {
                Some(true)
            } else if m.is_dir() {
                Some(false)
            } else {
                None
            }
        });
        cache.1.insert(path.to_path_buf(), kind);
        kind
    })
}

fn is_file(path: &Path) -> bool {
    path_kind(path) == Some(true)
}

fn is_dir(path: &Path) -> bool {
    path_kind(path) == Some(false)
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript path aliases
// ---------------------------------------------------------------------------

/// Largest `tsconfig.json` / `jsconfig.json` read for path aliases.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Aliases used without any config by Vite, Nuxt and Create React App
/// templates: `@/x` and `~/x` mean `src/x` (or `x` at the package root).
const DEFAULT_ALIASES: &[(&str, &[&str])] = &[
    ("@/*", &["src/*", "*"]),
    ("~/*", &["src/*", "*"]),
    // SvelteKit's `$lib` and Docusaurus's `@site`, set by the framework.
    ("$lib/*", &["src/lib/*"]),
    ("$lib", &["src/lib"]),
    ("@site/*", &["*"]),
];

/// Path aliases in force below one directory.
#[derive(Debug, Default)]
struct AliasScope {
    /// Directory holding the config or `package.json`.
    dir: PathBuf,
    /// Directory alias targets are relative to (`baseUrl`, else `dir`).
    base: PathBuf,
    /// Whether `baseUrl` was set, so bare specifiers resolve against it.
    base_url: bool,
    /// `paths` entries: pattern → target paths (already joined to their
    /// base directory), in config order.
    paths: Vec<(String, Vec<String>)>,
}

/// Path aliases from every `tsconfig.json`, `jsconfig.json` and
/// `package.json` in the repository, deepest directory first.
#[derive(Debug, Default)]
struct Aliases {
    scopes: Vec<AliasScope>,
    /// Every file in the repository; aliases only ever resolve to one of
    /// these, so a config cannot point outside the project.
    known: HashSet<PathBuf>,
    /// Every named `package.json` in the repository, by `name`: imports
    /// of these are the repository's own workspace packages.
    packages: HashMap<String, LocalPackage>,
}

/// One of the repository's own packages.
#[derive(Debug)]
struct LocalPackage {
    /// Directory holding its `package.json`.
    dir: PathBuf,
    /// The parsed `package.json`.
    manifest: serde_json::Value,
}

/// Export conditions tried first: those naming source before those naming
/// built output, which is often not in the repository.
const CONDITIONS: &[&str] = &[
    "source",
    "development",
    "import",
    "module",
    "default",
    "require",
    "node",
    "browser",
    "types",
];

/// Folders built output usually goes to, mapped back to `src` when the
/// built file is not in the repository.
const BUILD_DIRS: &[&str] = &["dist", "build", "lib", "out", "esm", "cjs"];

impl Aliases {
    fn from_files(files: &[PathBuf]) -> Self {
        let mut scopes: BTreeMap<PathBuf, AliasScope> = BTreeMap::new();
        let mut packages = HashMap::new();
        for file in files {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !matches!(name, "tsconfig.json" | "jsconfig.json" | "package.json") {
                continue;
            }
            let path = normalize(file);
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let scope = scopes.entry(dir.clone()).or_insert_with(|| AliasScope {
                dir: dir.clone(),
                base: dir.clone(),
                ..AliasScope::default()
            });
            if name == "package.json" {
                if let Some(manifest) = package_manifest(&path) {
                    if let Some(name) = manifest_name(&manifest) {
                        packages.insert(
                            name,
                            LocalPackage {
                                dir: dir.clone(),
                                manifest,
                            },
                        );
                    }
                }
            } else {
                read_ts_config(&path, scope);
            }
        }

        let mut scopes: Vec<AliasScope> = scopes.into_values().collect();
        for scope in &mut scopes {
            for (pattern, targets) in DEFAULT_ALIASES {
                if !scope.paths.iter().any(|(p, _)| p == pattern) {
                    let targets = targets
                        .iter()
                        .map(|t| scope.dir.join(t).to_string_lossy().into_owned())
                        .collect();
                    scope.paths.push((pattern.to_string(), targets));
                }
            }
        }
        scopes.sort_by_key(|s| std::cmp::Reverse(s.dir.components().count()));

        Self {
            scopes,
            known: files.iter().map(|f| normalize(f)).collect(),
            packages,
        }
    }

    /// Whether a non-relative specifier names the repository's own code: it
    /// matches a path alias in force for `file` or a workspace package.
    fn is_local(&self, file: &Path, specifier: &str) -> bool {
        // No npm package name starts with `@/` or `~/`: these are always
        // aliases of the project's own code, configured or not.
        if specifier.starts_with("@/") || specifier.starts_with("~/") {
            return true;
        }
        self.package_for(specifier).is_some()
            || self
                .scopes
                .iter()
                .filter(|s| file.starts_with(&s.dir))
                .flat_map(|s| &s.paths)
                .any(|(pattern, _)| match pattern.strip_suffix('*') {
                    Some(prefix) => !prefix.is_empty() && specifier.starts_with(prefix),
                    None => pattern == specifier,
                })
    }

    /// Resolve a non-relative specifier through the aliases of the nearest
    /// enclosing configs.
    fn resolve(&self, file: &Path, specifier: &str) -> Option<PathBuf> {
        for scope in self.scopes.iter().filter(|s| file.starts_with(&s.dir)) {
            for (pattern, targets) in &scope.paths {
                let rest = match pattern.strip_suffix('*') {
                    Some(prefix) => match specifier.strip_prefix(prefix) {
                        Some(rest) => rest,
                        None => continue,
                    },
                    None if pattern == specifier => "",
                    None => continue,
                };
                for target in targets {
                    let target = target.replacen('*', rest, 1);
                    if let Some(found) = self.known_file(Path::new(""), &target) {
                        return Some(found);
                    }
                }
            }
            if scope.base_url {
                if let Some(found) = self.known_file(&scope.base, specifier) {
                    return Some(found);
                }
            }
        }
        self.resolve_package(specifier)
    }

    /// The workspace package `specifier` imports, with the subpath after
    /// its name (`.` for the package itself, else `./rest`).
    fn package_for(&self, specifier: &str) -> Option<(&LocalPackage, String)> {
        self.packages
            .iter()
            .filter_map(|(name, package)| {
                let rest = specifier.strip_prefix(name.as_str())?;
                if rest.is_empty() {
                    Some((name.len(), package, ".".to_string()))
                } else {
                    let rest = rest.strip_prefix('/')?;
                    Some((name.len(), package, format!("./{rest}")))
                }
            })
            .max_by_key(|(len, _, _)| *len)
            .map(|(_, package, subpath)| (package, subpath))
    }

    /// Resolve an import of a workspace package to a file: its `exports`
    /// entry, then `source` / `module` / `main` / `types`, then
    /// `src/index` or `index`. Built paths such as `dist/index.js` fall
    /// back to the same path under `src/` when the build is not checked in.
    fn resolve_package(&self, specifier: &str) -> Option<PathBuf> {
        let (package, subpath) = self.package_for(specifier)?;
        let manifest = &package.manifest;
        let mut targets = manifest
            .get("exports")
            .map(|exports| export_targets(exports, &subpath))
            .unwrap_or_default();
        let has_exports = manifest.get("exports").is_some();
        if subpath == "." {
            if !has_exports {
                for field in ["source", "module", "main", "types", "typings"] {
                    if let Some(target) = manifest.get(field).and_then(|v| v.as_str()) {
                        targets.push(target.to_string());
                    }
                }
            }
            targets.extend(["./src/index".to_string(), "./index".to_string()]);
        } else if !has_exports {
            let rest = &subpath[2..];
            targets.extend([format!("./{rest}"), format!("./src/{rest}")]);
        }
        targets.iter().find_map(|target| {
            self.known_file(&package.dir, target).or_else(|| {
                let source = source_for_built(target)?;
                self.known_file(&package.dir, &source)
            })
        })
    }

    fn known_file(&self, base: &Path, target: &str) -> Option<PathBuf> {
        resolve_js_ts_path(base, target).filter(|p| self.known.contains(p))
    }
}

/// A parsed `package.json`, unless it is too large or invalid.
fn package_manifest(path: &Path) -> Option<serde_json::Value> {
    let small = fs::metadata(path).is_ok_and(|m| m.len() <= MAX_CONFIG_BYTES);
    if !small {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The `name` field of a `package.json`, when it has one.
fn manifest_name(manifest: &serde_json::Value) -> Option<String> {
    let name = manifest.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Files an `exports` field maps `subpath` (`.` or `./x`) to, most
/// preferred first. Handles a plain string, condition objects, arrays,
/// subpath maps and `*` patterns.
fn export_targets(exports: &serde_json::Value, subpath: &str) -> Vec<String> {
    let is_subpath_map = exports
        .as_object()
        .is_some_and(|map| map.keys().any(|k| k.starts_with('.')));
    if !is_subpath_map {
        return if subpath == "." {
            condition_targets(exports)
        } else {
            Vec::new()
        };
    }
    let Some(map) = exports.as_object() else {
        return Vec::new();
    };
    if let Some(entry) = map.get(subpath) {
        return condition_targets(entry);
    }
    // The longest matching `*` pattern wins, as in Node.
    let best = map
        .iter()
        .filter_map(|(key, entry)| {
            let (prefix, suffix) = key.split_once('*')?;
            let middle = subpath.strip_prefix(prefix)?.strip_suffix(suffix)?;
            Some((prefix.len(), middle, entry))
        })
        .max_by_key(|(len, _, _)| *len);
    match best {
        Some((_, middle, entry)) => condition_targets(entry)
            .into_iter()
            .map(|t| t.replace('*', middle))
            .collect(),
        None => Vec::new(),
    }
}

/// Every path in an export entry, preferred conditions first.
fn condition_targets(entry: &serde_json::Value) -> Vec<String> {
    match entry {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(items) => items.iter().flat_map(condition_targets).collect(),
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|k| {
                CONDITIONS
                    .iter()
                    .position(|c| c == k)
                    .unwrap_or(CONDITIONS.len())
            });
            keys.into_iter()
                .flat_map(|k| condition_targets(&map[k]))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// `./dist/x.js` → `./src/x`: where the source of a built file usually
/// lives. `None` when the path is not under a build folder.
fn source_for_built(target: &str) -> Option<String> {
    let trimmed = target.strip_prefix("./").unwrap_or(target);
    let (first, rest) = trimmed.split_once('/')?;
    if !BUILD_DIRS.contains(&first) {
        return None;
    }
    let rest = [".d.ts", ".d.mts", ".d.cts", ".js", ".mjs", ".cjs"]
        .iter()
        .find_map(|ext| rest.strip_suffix(ext))
        .unwrap_or(rest);
    Some(format!("./src/{rest}"))
}

/// Read `compilerOptions.baseUrl` and `compilerOptions.paths` from a
/// TypeScript or JavaScript project config. Unreadable or invalid configs
/// are ignored.
fn read_ts_config(path: &Path, scope: &mut AliasScope) {
    let small = fs::metadata(path).is_ok_and(|m| m.len() <= MAX_CONFIG_BYTES);
    if !small {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)) else {
        return;
    };
    let Some(options) = json.get("compilerOptions") else {
        return;
    };
    if let Some(base_url) = options.get("baseUrl").and_then(|b| b.as_str()) {
        scope.base = normalize(&scope.dir.join(base_url));
        scope.base_url = true;
    }
    if let Some(paths) = options.get("paths").and_then(|p| p.as_object()) {
        for (pattern, targets) in paths {
            if scope.paths.iter().any(|(p, _)| p == pattern) {
                continue;
            }
            let targets = match targets.as_array() {
                Some(list) => list
                    .iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| scope.base.join(t).to_string_lossy().into_owned())
                    .collect(),
                None => Vec::new(),
            };
            scope.paths.push((pattern.clone(), targets));
        }
    }
}

/// Turn JSON with comments and trailing commas (as `tsconfig.json` allows)
/// into plain JSON.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(next) = chars.next() {
                    out.push(next);
                    if next == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    } else if next == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for next in chars.by_ref() {
                    if last == '*' && next == '/' {
                        break;
                    }
                    last = next;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    drop_trailing_commas(&out)
}

/// Remove commas directly before `}` or `]`, outside strings.
fn drop_trailing_commas(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut out = String::with_capacity(json.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if c == '\\' {
                out.push(c);
                i += 1;
                if let Some(&escaped) = chars.get(i) {
                    out.push(escaped);
                }
            } else {
                in_string = c != '"';
                out.push(c);
            }
            i += 1;
            continue;
        }
        if c == ',' {
            let next = chars[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        in_string = c == '"';
        out.push(c);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Extensions whose imports CodeDna can follow.
/// Vue, Svelte and Astro components count for the scripts they hold.
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "vue", "svelte", "astro", "py", "rs",
];

/// Whether `file` is a source file the dependency graph understands.
fn is_source_file(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Parse dependencies for a single file, with no path aliases.
#[cfg(test)]
fn parse_dependencies(file: &Path) -> Vec<PathBuf> {
    let facts = parse_dependencies_with(
        file,
        &Aliases::default(),
        &HashSet::new(),
        &mut ImportTally::default(),
    );
    graph::from_facts([], &facts)
        .remove(file)
        .unwrap_or_default()
}

/// Parse the dependency facts of a single file based on its extension,
/// counting what happened to each import in `tally`.
fn parse_dependencies_with(
    file: &Path,
    aliases: &Aliases,
    python_packages: &HashSet<String>,
    tally: &mut ImportTally,
) -> Vec<DependencyFact> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let Ok(content) = fs::read_to_string(file) else {
        return Vec::new();
    };
    let Some(parent) = file.parent() else {
        return Vec::new();
    };
    tally.readable = true;

    match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "vue" | "svelte" | "astro" => {
            parse_js_ts_imports(file, parent, &content, aliases, tally)
        }
        "py" => parse_python_imports(file, parent, &content, python_packages, tally),
        "rs" => parse_rust_imports(file, parent, &content, tally),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript parser
// ---------------------------------------------------------------------------

/// Parse `import` and `require` statements and return resolved paths.
///
/// Patterns matched:
/// - `import ... from './path'`
/// - `import ... from "../path"`
/// - `require('./path')`
/// - `import('./path')` and `import './path'`
/// - aliased paths such as `@/components/x`, via `tsconfig.json` /
///   `jsconfig.json` `paths` and `baseUrl`
fn parse_js_ts_imports(
    file: &Path,
    parent: &Path,
    content: &str,
    aliases: &Aliases,
    tally: &mut ImportTally,
) -> Vec<DependencyFact> {
    let mut facts = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut seen_facts: HashSet<(&str, DependencyKind, &str)> = HashSet::new();

    // Set when a line ends in `import(` or `require(`, so the path on the
    // next line belongs to it.
    let mut open_call: Option<JsTrigger> = None;
    // The `import {` or `export {` whose `from` is on a later line.
    let mut open_list: Option<JsStatement> = None;
    for (number, line) in content.lines().enumerate() {
        let mut trimmed = line.trim();
        // A leading `/* webpackChunkName: "x" */` hint is not the path.
        if let Some(rest) = trimmed.strip_prefix("/*") {
            if let Some(end) = rest.find("*/") {
                trimmed = rest[end + 2..].trim_start();
            }
        }

        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        let continued = open_call
            .take()
            .and_then(|trigger| Some((trigger, extract_first_quoted(trimmed)?)));
        open_call = if trimmed.ends_with("import(") {
            Some(JsTrigger::DynamicImport)
        } else if trimmed.ends_with("require(") {
            Some(JsTrigger::Require)
        } else {
            None
        };

        // The statement a `from "x"` on this line belongs to.
        let statement = JsStatement::starting(trimmed).or(open_list);
        if trimmed.contains('}') {
            open_list = None;
        }
        if let Some(started) = JsStatement::starting(trimmed) {
            if trimmed.contains('{') && !trimmed.contains('}') {
                open_list = Some(started);
            }
        }

        // Collect all quoted string literals on this line that look like
        // relative paths.
        for (trigger, specifier) in continued
            .into_iter()
            .chain(extract_quoted_specifiers(trimmed))
        {
            if specifier.is_empty() {
                continue;
            }
            let line = number + 1;
            // Template literals with `${...}` are built at runtime.
            if specifier.contains("${") {
                let kind = DependencyKind::PossibleDynamicLoad;
                if seen_facts.insert((specifier, kind, "js.template_import")) {
                    facts.push(
                        DependencyFact::new(
                            file,
                            specifier,
                            kind,
                            Certainty::Possibility,
                            "js.template_import",
                        )
                        .line(line)
                        .span_in(content, specifier)
                        .resolution(Resolution::Unsupported),
                    );
                }
                continue;
            }
            let resolved = if is_relative(specifier) {
                resolve_js_ts_path(parent, specifier)
            } else {
                aliases.resolve(file, specifier)
            };
            let internal =
                resolved.is_some() || is_relative(specifier) || aliases.is_local(file, specifier);
            if seen.insert(specifier) {
                if internal {
                    tally.internal(line, specifier, resolved.is_some());
                } else {
                    tally.external += 1;
                }
            }

            let (kind, rule) = trigger.kind(statement);
            if !seen_facts.insert((specifier, kind, rule)) {
                continue;
            }
            let mut fact = DependencyFact::new(file, specifier, kind, Certainty::Direct, rule)
                .line(line)
                .span_in(content, specifier);
            if trigger == JsTrigger::DynamicImport {
                fact = fact.condition("dynamic");
            }
            facts.push(match resolved {
                Some(resolved) => fact.resolved(vec![resolved]),
                None if internal => fact.resolution(Resolution::Unresolved),
                None => fact.resolution(Resolution::External),
            });
        }
    }

    for (literal, load) in runtime_loaded_files(file, content) {
        if let Some(resolved) = resolve_js_ts_path(parent, literal) {
            let (kind, rule) = match load {
                RuntimeLoad::Url => (DependencyKind::ResourceReference, "js.new_url"),
                RuntimeLoad::Worker => (DependencyKind::WorkerEntry, "js.worker"),
                RuntimeLoad::Process => (DependencyKind::ProcessEntry, "js.process"),
            };
            let mut fact = DependencyFact::new(file, literal, kind, Certainty::Inferred, rule)
                .span_in(content, literal)
                .resolved(vec![resolved]);
            if let Some([start, _]) = fact.evidence.span {
                fact = fact.line(content[..start].matches('\n').count() + 1);
            }
            facts.push(fact);
        }
    }

    facts
}

/// What introduced a quoted specifier on a JS/TS line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsTrigger {
    /// `... from "x"`
    From,
    /// `require("x")`
    Require,
    /// `import("x")`
    DynamicImport,
    /// `import "x"`
    SideEffect,
}

/// The statement a `from "x"` closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsStatement {
    Import,
    ImportType,
    Export,
    ExportType,
}

impl JsStatement {
    /// The statement `line` starts, if it starts an import or export.
    fn starting(line: &str) -> Option<Self> {
        let keyword = |rest: &str| rest.starts_with([' ', '{', '*', '\t']);
        if let Some(rest) = line.strip_prefix("import") {
            if let Some(after) = rest.trim_start().strip_prefix("type") {
                if keyword(after) && rest.starts_with(char::is_whitespace) {
                    return Some(JsStatement::ImportType);
                }
            }
            return keyword(rest).then_some(JsStatement::Import);
        }
        if let Some(rest) = line.strip_prefix("export") {
            if let Some(after) = rest.trim_start().strip_prefix("type") {
                if keyword(after) && rest.starts_with(char::is_whitespace) {
                    return Some(JsStatement::ExportType);
                }
            }
            return keyword(rest).then_some(JsStatement::Export);
        }
        None
    }
}

impl JsTrigger {
    /// The kind of dependency and the rule id for a specifier this trigger
    /// introduced, inside `statement`.
    fn kind(self, statement: Option<JsStatement>) -> (DependencyKind, &'static str) {
        match self {
            JsTrigger::Require => (DependencyKind::RuntimeImport, "js.require"),
            JsTrigger::DynamicImport => (DependencyKind::RuntimeImport, "js.dynamic_import"),
            JsTrigger::SideEffect => (DependencyKind::RuntimeImport, "js.side_effect_import"),
            JsTrigger::From => match statement {
                Some(JsStatement::ImportType) => (DependencyKind::TypeOnlyImport, "js.import_type"),
                Some(JsStatement::ExportType) => {
                    (DependencyKind::TypeOnlyImport, "js.export_type_from")
                }
                Some(JsStatement::Export) => (DependencyKind::ReExport, "js.export_from"),
                Some(JsStatement::Import) | None => (DependencyKind::RuntimeImport, "js.import"),
            },
        }
    }
}

/// How a JS/TS file starts another file at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeLoad {
    /// `new URL("./w.ts", import.meta.url)`
    Url,
    /// `new Worker("./w.js")`
    Worker,
    /// `spawn("tsx", ["job.ts"])`
    Process,
}

/// Modules whose exports start a file in another process or thread.
const LOADER_MODULES: &[&str] = &[
    "child_process",
    "node:child_process",
    "worker_threads",
    "node:worker_threads",
    "execa",
    "cross-spawn",
];

/// Calls that run a file in another process: `spawn("tsx", ["w.ts"])`.
const PROCESS_LOADERS: &[&str] = &[
    "spawn",
    "spawnSync",
    "fork",
    "exec",
    "execSync",
    "execFile",
    "execFileSync",
    "execa",
    "execaSync",
    "execaNode",
    "execaCommand",
    "execaCommandSync",
];

/// Process loaders that take one command line rather than an argument list.
const COMMAND_LINE_LOADERS: &[&str] = &["exec", "execSync", "execaCommand", "execaCommandSync"];

/// Constructors that run a file in a worker thread: `new Worker("./w.js")`.
const WORKER_CONSTRUCTORS: &[&str] = &["Worker", "SharedWorker"];

/// Source files a JS/TS file loads at runtime by name rather than by import:
///
/// - `new URL("./w.ts", import.meta.url)`, the bundler form for workers
/// - `new Worker("./w.js")` and `new SharedWorker(...)`, unless the file
///   defines or imports its own `Worker` from elsewhere
/// - `spawn`, `fork`, `exec` and the rest imported from `child_process`,
///   `execa` or `cross-spawn` (or called on a binding of those modules, or
///   `Bun.spawn`), with the file among their arguments:
///   `spawn("tsx", ["worker.ts"])`, `exec("tsx worker.ts")`
///
/// Calls are found in code only, so a path in a log message, a comment or
/// any other string is not a dependency, and neither is a call to a local
/// function that happens to share a loader's name.
fn runtime_loaded_files<'a>(file: &Path, content: &'a str) -> Vec<(&'a str, RuntimeLoad)> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let code = crate::code_text::code_only(&ext, content);
    let (loader_names, other_names) = imported_names(&code, content);
    let tokens = identifiers(&code);
    let bytes = code.as_bytes();

    let mut found = Vec::new();
    for (index, &(start, end)) in tokens.iter().enumerate() {
        let name = &code[start..end];
        let Some(open) = next_non_space(bytes, end).filter(|&i| bytes[i] == b'(') else {
            continue;
        };
        let args = open + 1..closing_paren(bytes, open);
        let receiver = receiver(&code, &tokens, index);
        // `new` before the callee, or before its receiver in `new wt.Worker(`.
        let callee = match receiver {
            Some(r) if !r.is_empty() => index - 1,
            _ => index,
        };
        let after_new = callee > 0 && {
            let (s, e) = tokens[callee - 1];
            &code[s..e] == "new" && code[e..tokens[callee].0].trim().is_empty()
        };

        let literals = string_literals(&code, content, args.clone());
        if after_new && name == "URL" && receiver.is_none() {
            if code[args].contains("import.meta.url") {
                found.extend(
                    literals
                        .first()
                        .filter(|l| l.0 == 0)
                        .map(|l| (l.1, RuntimeLoad::Url)),
                );
            }
        } else if after_new && WORKER_CONSTRUCTORS.contains(&name) {
            let is_loader = match receiver {
                Some(r) => loader_names.contains(r),
                None => {
                    loader_names.contains(name)
                        || !(other_names.contains(name) || declares(&code, &tokens, name))
                }
            };
            if is_loader {
                found.extend(
                    literals
                        .first()
                        .filter(|l| l.0 == 0)
                        .map(|l| (l.1, RuntimeLoad::Worker)),
                );
            }
        } else if !after_new && PROCESS_LOADERS.contains(&name) {
            let is_loader = match receiver {
                Some("Bun") => name == "spawn" || name == "spawnSync",
                Some(r) => loader_names.contains(r),
                None => loader_names.contains(name),
            };
            if !is_loader {
                continue;
            }
            for (_, literal) in literals {
                if COMMAND_LINE_LOADERS.contains(&name) {
                    found.extend(
                        literal
                            .split_whitespace()
                            .filter(|w| names_source_file(w))
                            .map(|w| (w, RuntimeLoad::Process)),
                    );
                } else if names_source_file(literal) {
                    found.push((literal, RuntimeLoad::Process));
                }
            }
        }
    }
    found.retain(|(literal, _)| names_source_file(literal));
    found
}

/// A JS/TS source file named by its extension: `"./worker.ts"` or
/// `'jobs/sync.mjs'`, but not URLs, absolute paths or command lines.
fn names_source_file(literal: &str) -> bool {
    let is_source = Path::new(literal)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["js", "mjs", "cjs", "ts", "mts", "cts", "tsx", "jsx"].contains(&e));
    is_source
        && !literal.contains("://")
        && !literal.contains("${")
        && !literal.contains(char::is_whitespace)
        && !literal.starts_with('/')
}

/// Byte ranges of the identifiers in `code`.
fn identifiers(code: &str) -> Vec<(usize, usize)> {
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_ident(bytes[i]) && !bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && is_ident(bytes[i]) {
                i += 1;
            }
            tokens.push((start, i));
        } else {
            i += 1;
        }
    }
    tokens
}

fn next_non_space(bytes: &[u8], from: usize) -> Option<usize> {
    (from..bytes.len()).find(|&i| !bytes[i].is_ascii_whitespace())
}

/// Index of the `)` closing the `(` at `open`, or the end of the text. String
/// contents are already blanked, so every parenthesis left is code.
fn closing_paren(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

/// `cp` in `cp.spawn(` or `cp?.spawn(`; `""` for a method on anything other
/// than a plain name (`/re/.exec(`, `a.b.exec(`); `None` for a bare call.
fn receiver<'a>(code: &'a str, tokens: &[(usize, usize)], index: usize) -> Option<&'a str> {
    let (start, _) = tokens[index];
    let before = code[..start].trim_end();
    let before = before.strip_suffix('.')?.trim_end();
    let before = before.strip_suffix('?').unwrap_or(before);
    let plain = index.checked_sub(1).and_then(|i| {
        let (s, e) = tokens[i];
        let start_of_chain = code[..s].trim_end();
        (e == before.len() && !start_of_chain.ends_with('.')).then(|| &code[s..e])
    });
    Some(plain.unwrap_or(""))
}

/// String literals inside `range` of `code`, as (offset of the opening quote
/// from the first non-space character, contents taken from `content`).
fn string_literals<'a>(
    code: &str,
    content: &'a str,
    range: std::ops::Range<usize>,
) -> Vec<(usize, &'a str)> {
    let bytes = code.as_bytes();
    let first = next_non_space(bytes, range.start).unwrap_or(range.start);
    let mut found = Vec::new();
    let mut i = range.start;
    while i < range.end.min(bytes.len()) {
        let quote = bytes[i];
        if matches!(quote, b'"' | b'\'' | b'`') {
            let Some(close) = (i + 1..range.end).find(|&j| bytes[j] == quote) else {
                break;
            };
            if let Some(text) = content.get(i + 1..close) {
                found.push((i - first, text));
            }
            i = close + 1;
        } else {
            i += 1;
        }
    }
    found
}

/// Names bound by imports, split into those from [`LOADER_MODULES`] and those
/// from anywhere else: `import { spawn } from "node:child_process"`,
/// `const cp = require("child_process")`, `import { Worker } from "./pool"`.
fn imported_names<'a>(code: &'a str, content: &str) -> (HashSet<&'a str>, HashSet<&'a str>) {
    let tokens = identifiers(code);
    let bytes = code.as_bytes();
    let mut loaders = HashSet::new();
    let mut others = HashSet::new();
    let mut i = 0;
    while i < bytes.len() {
        let quote = bytes[i];
        if !matches!(quote, b'"' | b'\'') {
            i += 1;
            continue;
        }
        let Some(close) = (i + 1..bytes.len()).find(|&j| bytes[j] == quote) else {
            break;
        };
        let module = content.get(i + 1..close).unwrap_or("");
        let before = code[..i].trim_end();
        // The keyword that starts the statement, and where the names end.
        let names = if before.ends_with("from") {
            let end = before.len() - 4;
            last_token(code, &tokens, end, &["import"]).map(|start| start..end)
        } else if let Some(call) = before.strip_suffix('(') {
            let call = call.trim_end();
            call.strip_suffix("require").and_then(|rest| {
                let end = rest.trim_end().strip_suffix('=')?.len();
                last_token(code, &tokens, end, &["const", "let", "var"]).map(|start| start..end)
            })
        } else {
            None
        };
        if let Some(range) = names {
            let target = if LOADER_MODULES.contains(&module) {
                &mut loaders
            } else {
                &mut others
            };
            for &(s, e) in tokens
                .iter()
                .filter(|t| t.0 >= range.start && t.1 <= range.end)
            {
                let name = &code[s..e];
                if !matches!(
                    name,
                    "import" | "const" | "let" | "var" | "type" | "typeof" | "as" | "default"
                ) {
                    target.insert(name);
                }
            }
        }
        i = close + 1;
    }
    (loaders, others)
}

/// Start of the last token before `end` that is one of `keywords`, if it
/// comes within a statement's reach (no `;` in between).
fn last_token(
    code: &str,
    tokens: &[(usize, usize)],
    end: usize,
    keywords: &[&str],
) -> Option<usize> {
    let &(start, _) = tokens
        .iter()
        .rev()
        .filter(|t| t.1 <= end)
        .find(|t| keywords.contains(&&code[t.0..t.1]))?;
    (!code[start..end].contains(';')).then_some(start)
}

/// Whether the file declares `name` itself: `class Worker`,
/// `function Worker`, `const Worker = ...`.
fn declares(code: &str, tokens: &[(usize, usize)], name: &str) -> bool {
    tokens.windows(2).any(|pair| {
        let (keyword, ident) = (pair[0], pair[1]);
        &code[ident.0..ident.1] == name
            && matches!(
                &code[keyword.0..keyword.1],
                "class" | "function" | "const" | "let" | "var"
            )
            && code[keyword.1..ident.0].trim().is_empty()
    })
}

/// Extract all single- or double-quoted string values from a line that are
/// preceded by `from`, `require`, or `import`.
fn extract_quoted_specifiers(line: &str) -> Vec<(JsTrigger, &str)> {
    let mut results = Vec::new();

    // We scan for from/require/import( patterns then pull the next quoted string.
    let triggers = [
        ("from ", JsTrigger::From),
        ("require(", JsTrigger::Require),
        ("import(", JsTrigger::DynamicImport),
        ("import ", JsTrigger::SideEffect),
    ];

    for (trigger, kind) in triggers {
        let mut search = line;
        while let Some(pos) = search.find(trigger) {
            let after = &search[pos + trigger.len()..].trim_start();

            if let Some(specifier) = extract_first_quoted(after) {
                results.push((kind, specifier));
            }

            search = &search[pos + trigger.len()..];
        }
    }

    results
}

/// Extract the first single- or double-quoted string at the beginning of `s`.
fn extract_first_quoted(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let quote = s.chars().next()?;

    if quote != '"' && quote != '\'' && quote != '`' {
        return None;
    }

    let inner = &s[1..];
    let end = inner.find(quote)?;
    Some(&inner[..end])
}

/// Resolve a JS/TS relative import specifier to an absolute path on disk.
///
/// Tries in order:
/// 1. Exact path (already has extension)
/// 2. `<path>.ts`, `<path>.tsx`, `<path>.js`, `<path>.jsx`, `<path>.mjs`,
///    appended so dotted names like `./user.service` keep their full name
/// 3. A `.js` specifier pointing at TypeScript (`./routes.js` → `routes.ts`)
/// 4. `<path>/index.ts`, `<path>/index.js`, etc.
fn resolve_js_ts_path(parent: &Path, specifier: &str) -> Option<PathBuf> {
    let base = parent.join(specifier);

    // 1. Exact path
    if is_file(&base) {
        return Some(normalize(&base));
    }

    // 2. With extension appended
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = append_extension(&base, ext);
        if is_file(&candidate) {
            return Some(normalize(&candidate));
        }
    }

    // 3. JavaScript extension standing in for TypeScript
    let js_like = base
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "js" | "jsx" | "mjs" | "cjs"));
    if js_like {
        for ext in &["ts", "tsx"] {
            let candidate = base.with_extension(ext);
            if is_file(&candidate) {
                return Some(normalize(&candidate));
            }
        }
    }

    // 4. Index file inside directory
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = base.join(format!("index.{ext}"));
        if is_file(&candidate) {
            return Some(normalize(&candidate));
        }
    }

    None
}

/// Append `.ext` to `path` without replacing a dotted suffix it already has.
fn append_extension(path: &Path, ext: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".");
    name.push(ext);
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------
// Python parser
// ---------------------------------------------------------------------------

/// Parse Python imports into dependency facts.
///
/// Patterns matched:
/// - `from .module import ...` and `from ..module import ...`
/// - `from . import module`
/// - `from package.module import ...` and `import package.module`
/// - dotted module paths in string literals, such as Django's
///   `ROOT_URLCONF = "project.urls"` or `include("app.urls")`
///
/// Third-party and stdlib imports find no matching file and become
/// `external` facts; local ones that match nothing stay as `unresolved`.
fn parse_python_imports(
    file: &Path,
    parent: &Path,
    content: &str,
    python_packages: &HashSet<String>,
    tally: &mut ImportTally,
) -> Vec<DependencyFact> {
    let mut facts = Vec::new();

    for (number, line) in python_logical_lines(content) {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with('#') {
            continue;
        }

        let rule = if trimmed.starts_with("from ") {
            "python.from_import"
        } else {
            "python.import"
        };
        for (module, found) in python_import_modules(parent, trimmed) {
            let local = module.starts_with('.')
                || python_packages.contains(module.split('.').next().unwrap_or(""));
            let fact = DependencyFact::new(
                file,
                module,
                DependencyKind::RuntimeImport,
                Certainty::Direct,
                rule,
            )
            .line(number + 1);
            facts.push(match found {
                Some(files) => {
                    tally.resolved += 1;
                    fact.resolved(files)
                }
                None if local => {
                    tally.internal(number + 1, module, false);
                    fact.resolution(Resolution::Unresolved)
                }
                None => {
                    tally.external += 1;
                    fact.resolution(Resolution::External)
                }
            });
        }

        for (literal, files, rule) in resolve_python_string_paths(parent, trimmed, python_packages)
        {
            facts.push(
                DependencyFact::new(
                    file,
                    literal,
                    DependencyKind::ConfiguredEntry,
                    Certainty::Inferred,
                    rule,
                )
                .line(number + 1)
                .resolved(files),
            );
        }
    }

    facts
}

/// Lines of Python source, numbered from 0, with a parenthesised
/// `from x import (` list or a backslash continuation joined onto the line
/// that starts the import.
fn python_logical_lines(content: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    // The import being joined, and whether it waits for a `)` rather than
    // for a line without a trailing backslash.
    let mut open: Option<(usize, String, bool)> = None;
    for (number, line) in content.lines().enumerate() {
        let code = line.split('#').next().unwrap_or("").trim();
        if let Some((start, mut joined, paren)) = open.take() {
            joined.push(' ');
            joined.push_str(code.trim_end_matches('\\'));
            let done = if paren {
                code.contains(')')
            } else {
                !code.ends_with('\\')
            };
            if done {
                out.push((start, joined));
            } else {
                open = Some((start, joined, paren));
            }
            continue;
        }
        let is_import = code.starts_with("from ") || code.starts_with("import ");
        let paren = code.contains('(') && !code.contains(')');
        if is_import && (paren || code.ends_with('\\')) {
            open = Some((number, code.trim_end_matches('\\').to_string(), paren));
        } else {
            out.push((number, line.to_string()));
        }
    }
    out.extend(open.map(|(start, joined, _)| (start, joined)));
    out
}

/// The modules one `from ... import ...` or `import ...` line imports, each
/// with the files it loads, or `None` when no enclosing directory has it.
fn python_import_modules<'a>(
    file_parent: &Path,
    line: &'a str,
) -> Vec<(&'a str, Option<Vec<PathBuf>>)> {
    let line = line.split('#').next().unwrap_or("").trim();

    if let Some(rest) = line.strip_prefix("from ") {
        let Some((module, names)) = rest.split_once(" import ") else {
            return Vec::new();
        };
        let names: Vec<&str> = names
            .split(',')
            .map(|n| n.trim().trim_start_matches('(').trim_end_matches(')'))
            .filter_map(|n| n.split_whitespace().next())
            .filter(|n| *n != "*" && *n != "\\")
            .collect();
        let module = module.trim();
        if module.is_empty() {
            return Vec::new();
        }
        return vec![(module, locate_python_module(file_parent, module, &names))];
    }

    if let Some(rest) = line.strip_prefix("import ") {
        return rest
            .split(',')
            .filter_map(|module| module.split_whitespace().next())
            .filter(|module| !module.starts_with('(') && !module.ends_with('('))
            .map(|module| (module, locate_python_module(file_parent, module, &[])))
            .collect();
    }

    Vec::new()
}

/// Resolve quoted dotted paths (`"app.views"`, `"app.auth.Backend"`) to the
/// modules they name. Frameworks load these by name: Django settings,
/// URL includes, Celery task names. A trailing attribute is dropped when the
/// full path is not a module.
fn resolve_python_string_paths<'a>(
    file_parent: &Path,
    line: &'a str,
    python_packages: &HashSet<String>,
) -> Vec<(&'a str, Vec<PathBuf>, &'static str)> {
    let mut deps = Vec::new();
    for literal in python_string_literals(line) {
        // A bare package name that is a Django app, as in `INSTALLED_APPS`.
        if python_packages.contains(literal) {
            let found = resolve_python_module_in(file_parent, literal, &[]);
            let is_app = found
                .iter()
                .any(|f| f.parent().is_some_and(|dir| is_django_app(dir, is_file)));
            if is_app {
                deps.push((literal, found, "python.installed_app"));
            }
            continue;
        }
        let segments: Vec<&str> = literal.split('.').collect();
        let is_dotted = segments.len() >= 2
            && segments.iter().all(|s| {
                s.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        if !is_dotted {
            continue;
        }
        for keep in (2..=segments.len()).rev().take(2) {
            let module = segments[..keep].join(".");
            let found = resolve_python_module_in(file_parent, &module, &[]);
            if found
                .iter()
                .any(|f| f.extension().is_some_and(|e| e == "py"))
            {
                deps.push((literal, found, "python.string_path"));
                break;
            }
        }
    }
    deps
}

/// Single- or double-quoted string literals on one line.
fn python_string_literals(line: &str) -> Vec<&str> {
    let mut literals = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find(['"', '\'']) {
        let quote = rest[start..].chars().next().unwrap_or('"');
        let after = &rest[start + 1..];
        let Some(end) = after.find(quote) else {
            break;
        };
        literals.push(&after[..end]);
        rest = &after[end + 1..];
    }
    literals
}

/// Find the files a module import loads, starting from the importing file.
///
/// Relative modules (leading dots) start from the file's own package and walk
/// up one directory per extra dot. Absolute modules are tried from each
/// enclosing directory, so `from app.core import db` in `backend/app/main.py`
/// finds `backend/app/core/db.py`.
fn resolve_python_module_in(file_parent: &Path, module: &str, names: &[&str]) -> Vec<PathBuf> {
    locate_python_module(file_parent, module, names).unwrap_or_default()
}

/// Like [`resolve_python_module_in`], but `None` when the module was not
/// found at all, as opposed to found with no files to load (a namespace
/// package).
fn locate_python_module(file_parent: &Path, module: &str, names: &[&str]) -> Option<Vec<PathBuf>> {
    let dot_count = module.chars().take_while(|&c| c == '.').count();
    let dotted = &module[dot_count..];

    if dot_count > 0 {
        return file_parent
            .ancestors()
            .nth(dot_count - 1)
            .and_then(|root| resolve_python_module(root, dotted, names));
    }

    // An absolute import resolves from any enclosing directory, or from its
    // `src/` (the src layout, where tests import the installed package).
    file_parent.ancestors().find_map(|root| {
        resolve_python_module(root, dotted, names)
            .or_else(|| resolve_python_module(&root.join("src"), dotted, names))
    })
}

/// Resolve a dotted module under `root`, or `None` when `root` lacks it.
///
/// Returns the module's file plus every package `__init__.py` passed on the
/// way, since Python runs those too. Imported names that are submodules
/// (`from . import utils`) resolve to their own files.
fn resolve_python_module(root: &Path, dotted: &str, names: &[&str]) -> Option<Vec<PathBuf>> {
    let segments: Vec<&str> = dotted.split('.').filter(|s| !s.is_empty()).collect();
    let mut dir = root.to_path_buf();
    let mut files = Vec::new();

    if segments.is_empty() {
        push_if_file(&mut files, &dir.join("__init__.py"));
    }

    for (i, segment) in segments.iter().enumerate() {
        let as_file = dir.join(format!("{segment}.py"));
        if i + 1 == segments.len() && is_file(&as_file) {
            files.push(normalize(&as_file));
            return Some(files);
        }

        dir = dir.join(segment);
        if !is_dir(&dir) {
            return None;
        }
        push_if_file(&mut files, &dir.join("__init__.py"));
    }

    for name in names {
        let as_file = dir.join(format!("{name}.py"));
        if is_file(&as_file) {
            files.push(normalize(&as_file));
        } else {
            push_if_file(&mut files, &dir.join(name).join("__init__.py"));
        }
    }

    Some(files)
}

/// Push `candidate` onto `files` when it exists.
fn push_if_file(files: &mut Vec<PathBuf>, candidate: &Path) {
    if is_file(candidate) {
        files.push(normalize(candidate));
    }
}

// ---------------------------------------------------------------------------
// Rust parser
// ---------------------------------------------------------------------------

/// Parse Rust module declarations into dependency facts.
///
/// Patterns matched:
/// - `mod name;` with any visibility (`pub`, `pub(crate)`, ...) and
///   leading attributes such as `#[cfg(test)]`
/// - `#[path = "file.rs"] mod name;`
///
/// A crate root or `mod.rs` loads `name.rs` / `name/mod.rs` next to it;
/// any other file `foo.rs` loads them from `foo/`.
fn parse_rust_imports(
    file: &Path,
    parent: &Path,
    content: &str,
    tally: &mut ImportTally,
) -> Vec<DependencyFact> {
    let module_dir = rust_module_dir(file, parent);

    let mut facts = Vec::new();
    let mut path_attr: Option<String> = None;

    for (number, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        if let Some(path) = rust_path_attr(trimmed) {
            path_attr = Some(path.to_string());
        }

        let decl = strip_rust_attributes(trimmed);
        if decl.is_empty() {
            continue;
        }

        // `mod module_name;`
        if let Some(module_name) = parse_rust_mod_decl(decl) {
            let (resolved, rule) = match path_attr.take() {
                Some(path) => {
                    let candidate = parent.join(path);
                    (
                        is_file(&candidate).then(|| normalize(&candidate)),
                        "rust.mod_path",
                    )
                }
                None => (resolve_rust_mod(&module_dir, module_name), "rust.mod"),
            };
            tally.internal(
                number + 1,
                &format!("mod {module_name};"),
                resolved.is_some(),
            );
            let fact = DependencyFact::new(
                file,
                module_name,
                DependencyKind::RuntimeImport,
                Certainty::Declaration,
                rule,
            )
            .line(number + 1)
            .span_in(content, module_name);
            facts.push(match resolved {
                Some(resolved) => fact.resolved(vec![resolved]),
                None => fact.resolution(Resolution::Unresolved),
            });
        } else {
            path_attr = None;
        }
    }

    facts
}

/// Directory holding the files of the modules a Rust file declares.
fn rust_module_dir(file: &Path, parent: &Path) -> PathBuf {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let crate_root = matches!(stem, "mod" | "lib" | "main")
        || matches!(dir_name, "bin" | "tests" | "examples" | "benches")
        || (stem == "build" && is_file(&parent.join("Cargo.toml")));
    if crate_root {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    }
}

/// The file named by a `#[path = "..."]` attribute at the start of a line.
fn rust_path_attr(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("#[path")?
        .trim_start()
        .strip_prefix('=')?;
    extract_first_quoted(rest)
}

/// Drop leading `#[...]` attributes from a line.
fn strip_rust_attributes(line: &str) -> &str {
    let mut rest = line.trim_start();
    while rest.starts_with("#[") {
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(i) => rest = rest[i + 1..].trim_start(),
            None => return "",
        }
    }
    rest
}

/// Extract module name from `mod name;` declarations.
///
/// Returns `None` for `mod name { ... }` (inline modules).
fn parse_rust_mod_decl(line: &str) -> Option<&str> {
    let mut line = line.trim_start();
    if let Some(rest) = line.strip_prefix("pub") {
        let rest = rest.trim_start();
        line = match rest.strip_prefix('(') {
            Some(inner) => inner.split_once(')')?.1.trim_start(),
            None => rest,
        };
    }
    let rest = line.strip_prefix("mod ")?;

    // Only handle `mod name;` not `mod name {`
    let (name, _) = rest.split_once(';')?;
    let name = name.trim();
    let name = name.strip_prefix("r#").unwrap_or(name);
    let valid = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then_some(name)
}

/// Resolve a Rust `mod name;` declaration to a file on disk.
///
/// Tries:
/// 1. `<dir>/<name>.rs`
/// 2. `<dir>/<name>/mod.rs`
fn resolve_rust_mod(dir: &Path, module_name: &str) -> Option<PathBuf> {
    let as_file = dir.join(format!("{module_name}.rs"));
    if is_file(&as_file) {
        return Some(normalize(&as_file));
    }

    let as_mod = dir.join(module_name).join("mod.rs");
    if is_file(&as_mod) {
        return Some(normalize(&as_mod));
    }

    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` if the specifier is a relative path (starts with `.`).
fn is_relative(specifier: &str) -> bool {
    specifier.starts_with('.')
}

/// Normalize a path by resolving `.` and `..` components without requiring
/// the path to exist (unlike `std::fs::canonicalize`).
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut components: Vec<std::path::Component> = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(components.last(), Some(std::path::Component::Normal(_))) {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            other => components.push(other),
        }
    }

    components.iter().collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent dir");
        }
        let mut f = File::create(path).expect("failed to create file");
        f.write_all(content.as_bytes())
            .expect("failed to write content");
    }

    // -----------------------------------------------------------------------
    // TypeScript / JavaScript
    // -----------------------------------------------------------------------

    #[test]
    fn follows_imports_in_vue_svelte_and_astro_components() {
        let dir = unique_temp_dir("codedna_dg_components");
        let toolbar = dir.join("toolbar.js");
        write_file(&toolbar, "export default [];");
        let history = dir.join("history.ts");
        write_file(&history, "export const h = 1;");
        let layout = dir.join("Layout.astro");
        write_file(&layout, "<slot />");

        let vue = dir.join("Editor.vue");
        write_file(
            &vue,
            "<template><div /></template>\n<script>\nimport toolbar from './toolbar'\n</script>",
        );
        let svelte = dir.join("Modal.svelte");
        write_file(
            &svelte,
            "<script lang=\"ts\">\n  import { h } from './history';\n</script>",
        );
        let page = dir.join("index.astro");
        write_file(
            &page,
            "---\nimport Layout from './Layout.astro';\n---\n<Layout />",
        );

        assert!(parse_dependencies(&vue).contains(&normalize(&toolbar)));
        assert!(parse_dependencies(&svelte).contains(&normalize(&history)));
        assert!(parse_dependencies(&page).contains(&normalize(&layout)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_files_loaded_at_runtime_by_name() {
        let dir = unique_temp_dir("codedna_dg_runtime");
        let worker = dir.join("worker.ts");
        write_file(&worker, "export {};");
        let thread = dir.join("thread.js");
        write_file(&thread, "export {};");

        let index = dir.join("index.ts");
        write_file(
            &index,
            "import { spawn } from \"node:child_process\";\n\
             spawn(process.execPath, [require.resolve(\"tsx/cli\"), \"worker.ts\"]);\n\
             new Worker(new URL('./thread.js', import.meta.url));\n\
             fetch(\"https://cdn.example.com/missing.js\");\n\
             load(\"./missing.ts\");",
        );

        let deps = parse_dependencies(&index);
        assert!(deps.contains(&normalize(&worker)));
        assert!(deps.contains(&normalize(&thread)));
        assert_eq!(deps.len(), 2, "{deps:?}");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_ts_relative_import_with_extension() {
        let dir = unique_temp_dir("codedna_dg_ts_ext");
        let routes = dir.join("routes.ts");
        write_file(&routes, "export const r = 1;");

        let server = dir.join("server.ts");
        write_file(&server, r#"import { r } from "./routes.ts";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.contains(&normalize(&routes)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_ts_relative_import_without_extension() {
        let dir = unique_temp_dir("codedna_dg_ts_no_ext");
        let routes = dir.join("routes.ts");
        write_file(&routes, "export const r = 1;");

        let server = dir.join("server.ts");
        write_file(&server, r#"import { r } from "./routes";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.contains(&normalize(&routes)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_ts_import_of_dotted_file_name() {
        let dir = unique_temp_dir("codedna_dg_ts_dotted");
        let service = dir.join("user.service.ts");
        write_file(&service, "export const s = 1;");
        let user = dir.join("user.ts");
        write_file(&user, "export const u = 1;");

        let controller = dir.join("user.controller.ts");
        write_file(&controller, r#"import { s } from "./user.service";"#);

        let deps = parse_dependencies(&controller);
        assert_eq!(deps, vec![normalize(&service)]);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_js_specifier_to_ts_file() {
        let dir = unique_temp_dir("codedna_dg_ts_js_spec");
        let routes = dir.join("routes.ts");
        write_file(&routes, "export const r = 1;");

        let server = dir.join("server.ts");
        write_file(&server, r#"import { r } from "./routes.js";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.contains(&normalize(&routes)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_js_require_call() {
        let dir = unique_temp_dir("codedna_dg_js_require");
        let utils = dir.join("utils.js");
        write_file(&utils, "module.exports = {};");

        let index = dir.join("index.js");
        write_file(&index, r#"const utils = require('./utils');"#);

        let deps = parse_dependencies(&index);
        assert!(deps.contains(&normalize(&utils)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_ts_index_file_in_subdirectory() {
        let dir = unique_temp_dir("codedna_dg_ts_index");
        let sub = dir.join("api");
        fs::create_dir_all(&sub).unwrap();
        let index = sub.join("index.ts");
        write_file(&index, "export const api = {};");

        let server = dir.join("server.ts");
        write_file(&server, r#"import { api } from "./api";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.contains(&normalize(&index)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_third_party_ts_imports() {
        let dir = unique_temp_dir("codedna_dg_ts_third_party");
        let server = dir.join("server.ts");
        write_file(&server, r#"import express from "express";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.is_empty());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deduplicates_repeated_imports() {
        let dir = unique_temp_dir("codedna_dg_ts_dedup");
        let utils = dir.join("utils.ts");
        write_file(&utils, "export const x = 1;");

        let server = dir.join("server.ts");
        write_file(
            &server,
            "import { x } from \"./utils\";\nimport { x } from \"./utils\";",
        );

        let deps = parse_dependencies(&server);
        let count = deps.iter().filter(|p| **p == normalize(&utils)).count();
        assert_eq!(count, 1);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_commented_out_imports() {
        let dir = unique_temp_dir("codedna_dg_ts_comments");
        let old = dir.join("old.ts");
        write_file(&old, "export const x = 1;");

        let server = dir.join("server.ts");
        write_file(&server, r#"// import { x } from "./old";"#);

        let deps = parse_dependencies(&server);
        assert!(deps.is_empty());

        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // Python
    // -----------------------------------------------------------------------

    #[test]
    fn resolves_python_relative_import_same_package() {
        let dir = unique_temp_dir("codedna_dg_py_rel");
        let utils = dir.join("utils.py");
        write_file(&utils, "def helper(): pass");

        let main = dir.join("main.py");
        write_file(&main, "from .utils import helper");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&utils)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_relative_import_parent_package() {
        let dir = unique_temp_dir("codedna_dg_py_parent");
        let shared = dir.join("shared.py");
        write_file(&shared, "def shared(): pass");

        let sub = dir.join("api");
        fs::create_dir_all(&sub).unwrap();
        let routes = sub.join("routes.py");
        write_file(&routes, "from ..shared import shared");

        let deps = parse_dependencies(&routes);
        assert!(deps.contains(&normalize(&shared)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_dotted_string_paths() {
        let dir = unique_temp_dir("codedna_dg_py_strings");
        let settings = dir.join("site").join("settings.py");
        write_file(&settings, "");
        let backends = dir.join("site").join("auth").join("backends.py");
        write_file(&backends, "class JWT: pass");
        let manage = dir.join("manage.py");
        write_file(
            &manage,
            "os.environ.setdefault(\"DJANGO_SETTINGS_MODULE\", \"site.settings\")\n\
             AUTH = ['site.auth.backends.JWT']\n\
             VERSION = \"1.0.2\"\nTEXT = 'see site.missing'\n",
        );

        let deps = parse_dependencies(&manage);
        assert!(deps.contains(&normalize(&settings)));
        assert!(deps.contains(&normalize(&backends)));
        assert_eq!(deps.len(), 2, "{deps:?}");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_absolute_import_from_enclosing_directory() {
        let dir = unique_temp_dir("codedna_dg_py_abs_local");
        let app = dir.join("backend").join("app");
        let init = app.join("__init__.py");
        write_file(&init, "");
        let core_init = app.join("core").join("__init__.py");
        write_file(&core_init, "");
        let config = app.join("core").join("config.py");
        write_file(&config, "settings = 1");

        let main = app.join("main.py");
        write_file(&main, "from app.core.config import settings");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&config)));
        assert!(deps.contains(&normalize(&core_init)));
        assert!(deps.contains(&normalize(&init)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_imported_submodules() {
        let dir = unique_temp_dir("codedna_dg_py_submodule");
        let routes = dir.join("api").join("routes");
        let items = routes.join("items.py");
        write_file(&items, "router = 1");
        let users = routes.join("users.py");
        write_file(&users, "router = 1");

        let main = dir.join("api").join("main.py");
        write_file(
            &main,
            "from api.routes import items, users\nimport api.routes.items",
        );

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&items)));
        assert!(deps.contains(&normalize(&users)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_from_dot_import_module() {
        let dir = unique_temp_dir("codedna_dg_py_dot_module");
        let utils = dir.join("utils.py");
        write_file(&utils, "def helper(): pass");

        let main = dir.join("main.py");
        write_file(&main, "from . import utils");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&utils)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_python_imports_split_across_lines() {
        let dir = unique_temp_dir("codedna_dg_py_multiline");
        let routes = dir.join("api").join("routes");
        let groups = routes.join("groups.py");
        write_file(&groups, "router = 1");
        let users = routes.join("users.py");
        write_file(&users, "router = 1");
        let items = routes.join("items.py");
        write_file(&items, "router = 1");

        let main = dir.join("api").join("main.py");
        write_file(
            &main,
            "from api.routes import (\n    groups,  # admin\n    users,\n)\nfrom api.routes import \\\n    items\nx = (1,\n 2)",
        );

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&groups)));
        assert!(deps.contains(&normalize(&users)));
        assert!(deps.contains(&normalize(&items)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_absolute_python_imports() {
        let dir = unique_temp_dir("codedna_dg_py_abs");
        let main = dir.join("main.py");
        write_file(&main, "import os\nimport sys\nfrom fastapi import FastAPI");

        let deps = parse_dependencies(&main);
        assert!(deps.is_empty());

        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // Rust
    // -----------------------------------------------------------------------

    #[test]
    fn resolves_rust_mod_declaration_to_sibling_file() {
        let dir = unique_temp_dir("codedna_dg_rs_mod");
        let scanner = dir.join("scanner.rs");
        write_file(&scanner, "pub fn scan() {}");

        let main = dir.join("main.rs");
        write_file(&main, "mod scanner;\nfn main() {}");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&scanner)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_rust_mod_declaration_to_mod_rs() {
        let dir = unique_temp_dir("codedna_dg_rs_mod_rs");
        let sub = dir.join("api");
        fs::create_dir_all(&sub).unwrap();
        let mod_rs = sub.join("mod.rs");
        write_file(&mod_rs, "pub fn handler() {}");

        let main = dir.join("main.rs");
        write_file(&main, "mod api;\nfn main() {}");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&mod_rs)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_pub_mod_declaration() {
        let dir = unique_temp_dir("codedna_dg_rs_pub_mod");
        let cli = dir.join("cli.rs");
        write_file(&cli, "pub fn run() {}");

        let main = dir.join("main.rs");
        write_file(&main, "pub mod cli;\nfn main() {}");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&cli)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ignores_inline_rust_mod_blocks() {
        let dir = unique_temp_dir("codedna_dg_rs_inline");
        let main = dir.join("main.rs");
        write_file(&main, "mod tests {\n    fn test_it() {}\n}");

        let deps = parse_dependencies(&main);
        assert!(deps.is_empty());

        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // build_dependency_graph wiring
    // -----------------------------------------------------------------------

    #[test]
    fn build_graph_returns_entry_for_every_input_file() {
        let dir = unique_temp_dir("codedna_dg_wiring");
        let a = dir.join("a.ts");
        let b = dir.join("b.ts");
        write_file(&a, r#"import { x } from "./b";"#);
        write_file(&b, "export const x = 1;");

        let graph = build_dependency_graph(&[a.clone(), b.clone()]);

        assert!(graph.contains_key(&a));
        assert!(graph.contains_key(&b));
        assert!(graph[&a].contains(&normalize(&b)));
        assert!(graph[&b].is_empty());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn build_graph_leaves_out_non_source_files() {
        let dir = unique_temp_dir("codedna_dg_non_source");
        let main = dir.join("main.rs");
        let readme = dir.join("README.md");
        let config = dir.join("config.yml");
        write_file(&main, "fn main() {}");
        write_file(&readme, "# Hello");
        write_file(&config, "key: value");

        let graph = build_dependency_graph(&[main.clone(), readme.clone(), config.clone()]);

        assert_eq!(graph.len(), 1);
        assert!(graph.contains_key(&normalize(&main)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn build_graph_handles_unreadable_file_gracefully() {
        let missing = PathBuf::from("/no/such/file/ever/codedna_missing.ts");
        let graph = build_dependency_graph(std::slice::from_ref(&missing));

        assert!(graph.contains_key(&missing));
        assert!(graph[&missing].is_empty());
    }

    // -----------------------------------------------------------------------
    // Path aliases
    // -----------------------------------------------------------------------

    #[test]
    fn follows_tsconfig_path_aliases_and_base_url() {
        let dir = unique_temp_dir("codedna_dg_alias");
        let config = dir.join("tsconfig.json");
        write_file(
            &config,
            r#"{
  // comments and trailing commas are allowed
  "compilerOptions": {
    "baseUrl": ".", /* block */
    "paths": { "@lib/*": ["lib/*",], "@config": ["config/index.ts"] },
  },
}"#,
        );
        let app = dir.join("src/app.ts");
        let util = dir.join("lib/util.ts");
        let conf = dir.join("config/index.ts");
        let shared = dir.join("src/shared.ts");
        let button = dir.join("src/components/Button.tsx");
        write_file(
            &app,
            "import { u } from '@lib/util';\nimport c from \"@config\";\nimport s from 'src/shared';\nimport B from '@/components/Button';\nimport React from 'react';\n",
        );
        for f in [&util, &conf, &shared, &button] {
            write_file(f, "export {}");
        }

        let files = [
            config,
            app.clone(),
            util.clone(),
            conf.clone(),
            shared.clone(),
            button.clone(),
        ];
        let graph = build_dependency_graph(&files);
        let deps = &graph[&normalize(&app)];

        for f in [&util, &conf, &shared, &button] {
            assert!(deps.contains(&normalize(f)), "missing {f:?} in {deps:?}");
        }
        assert_eq!(deps.len(), 4);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_default_at_alias_without_config() {
        let dir = unique_temp_dir("codedna_dg_alias_default");
        let pkg = dir.join("package.json");
        write_file(&pkg, "{}");
        let main = dir.join("src/main.tsx");
        let view = dir.join("src/views/Home.vue.ts");
        let store = dir.join("src/store/index.ts");
        write_file(
            &main,
            "import Home from '@/views/Home.vue';\nimport '~/store';\n",
        );
        write_file(&view, "export {}");
        write_file(&store, "export {}");

        let graph = build_dependency_graph(&[pkg, main.clone(), view.clone(), store.clone()]);
        let deps = &graph[&normalize(&main)];

        assert!(deps.contains(&normalize(&view)));
        assert!(deps.contains(&normalize(&store)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_dynamic_imports_split_across_lines() {
        let dir = unique_temp_dir("codedna_dg_split_import");
        let main = dir.join("src/main.ts");
        let template = dir.join("src/template.js");
        write_file(
            &main,
            "const t = await import(\n  /* webpackChunkName: \"template\" */ \"./template\"\n);\n",
        );
        write_file(&template, "export default 1;");

        let graph = build_dependency_graph(&[main.clone(), template.clone()]);

        assert!(graph[&normalize(&main)].contains(&normalize(&template)));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_sveltekit_lib_alias() {
        let dir = unique_temp_dir("codedna_dg_lib_alias");
        let pkg = dir.join("package.json");
        let page = dir.join("src/routes/+page.ts");
        let util = dir.join("src/lib/format.ts");
        write_file(&pkg, "{}");
        write_file(&page, "import { f } from '$lib/format';\n");
        write_file(&util, "export const f = 1;");

        let graph = build_dependency_graph(&[pkg, page.clone(), util.clone()]);

        assert!(graph[&normalize(&page)].contains(&normalize(&util)));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_python_src_layout() {
        let dir = unique_temp_dir("codedna_dg_src_layout");
        let test = dir.join("tests/test_core.py");
        let init = dir.join("src/pkg/__init__.py");
        let core = dir.join("src/pkg/core.py");
        write_file(&test, "from pkg.core import run\n");
        write_file(&init, "");
        write_file(&core, "def run(): pass\n");

        let graph = build_dependency_graph(&[test.clone(), init, core.clone()]);

        assert!(graph[&normalize(&test)].contains(&normalize(&core)));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn django_app_without_apps_py_adds_no_such_file() {
        let dir = unique_temp_dir("codedna_dg_django_no_apps");
        let init = dir.join("blog/__init__.py");
        let models = dir.join("blog/models.py");
        write_file(&init, "");
        write_file(&models, "class Post: pass\n");

        let graph = build_dependency_graph(&[init.clone(), models.clone()]);

        assert!(graph[&normalize(&init)].contains(&normalize(&models)));
        assert!(!graph.contains_key(&normalize(&dir.join("blog/apps.py"))));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn links_installed_django_apps_to_their_autoloaded_modules() {
        let dir = unique_temp_dir("codedna_dg_django_apps");
        let settings = dir.join("site/settings.py");
        let init = dir.join("shop/__init__.py");
        let apps = dir.join("shop/apps.py");
        let models = dir.join("shop/models.py");
        let tags = dir.join("shop/templatetags/prices.py");
        let command = dir.join("shop/management/commands/restock.py");
        write_file(&settings, "INSTALLED_APPS = [\n    \"shop\",\n]\n");
        write_file(&init, "");
        write_file(&apps, "class ShopConfig: pass\n");
        write_file(&models, "class Item: pass\n");
        write_file(&tags, "");
        write_file(&command, "");
        let files = [
            settings.clone(),
            init.clone(),
            apps,
            models.clone(),
            tags.clone(),
            command.clone(),
        ];

        let graph = build_dependency_graph(&files);

        assert!(graph[&normalize(&settings)].contains(&normalize(&init)));
        let linked = &graph[&normalize(&init)];
        for file in [&models, &tags, &command] {
            assert!(linked.contains(&normalize(file)), "{file:?} not linked");
        }
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn aliases_never_resolve_outside_the_scanned_files() {
        let dir = unique_temp_dir("codedna_dg_alias_outside");
        let repo = dir.join("repo");
        let config = repo.join("tsconfig.json");
        write_file(
            &config,
            r#"{"compilerOptions": {"paths": {"@x/*": ["../outside/*"]}}}"#,
        );
        let outside = dir.join("outside/secret.ts");
        write_file(&outside, "export {}");
        let app = repo.join("app.ts");
        write_file(&app, "import s from '@x/secret';\n");

        let graph = build_dependency_graph(&[config, app.clone()]);

        assert!(graph[&normalize(&app)].is_empty());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follows_side_effect_imports() {
        let dir = unique_temp_dir("codedna_dg_side_effect");
        let polyfill = dir.join("polyfill.ts");
        write_file(&polyfill, "export {}");
        let main = dir.join("main.ts");
        write_file(&main, "import './polyfill';\n");

        let deps = parse_dependencies(&main);
        assert!(deps.contains(&normalize(&polyfill)));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn strips_jsonc_without_touching_strings() {
        let json = strip_jsonc("{\"a\": \"http://x/*y*/\", /* c */ \"b\": [1, 2,], // d\n}");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["a"], "http://x/*y*/");
        assert_eq!(value["b"], serde_json::json!([1, 2]));
    }

    // -----------------------------------------------------------------------
    // Rust module layout
    // -----------------------------------------------------------------------

    #[test]
    fn resolves_rust_child_modules_of_non_mod_files() {
        let dir = unique_temp_dir("codedna_dg_rs_children");
        let scanner = dir.join("src/scanner.rs");
        write_file(
            &scanner,
            "pub(crate) mod walk;\n#[cfg(test)]\nmod tests;\n#[path = \"extra/other.rs\"]\npub mod other;\n#[cfg(feature = \"x\")] pub(super) mod fast;\n",
        );
        let walk = dir.join("src/scanner/walk.rs");
        let tests = dir.join("src/scanner/tests.rs");
        let other = dir.join("src/extra/other.rs");
        let fast = dir.join("src/scanner/fast/mod.rs");
        for f in [&walk, &tests, &other, &fast] {
            write_file(f, "");
        }

        let deps = parse_dependencies(&scanner);

        for f in [&walk, &tests, &other, &fast] {
            assert!(deps.contains(&normalize(f)), "missing {f:?} in {deps:?}");
        }

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rust_integration_tests_load_siblings() {
        let dir = unique_temp_dir("codedna_dg_rs_tests");
        let cli = dir.join("tests/cli.rs");
        write_file(&cli, "mod common;\n");
        let common = dir.join("tests/common/mod.rs");
        write_file(&common, "");

        let deps = parse_dependencies(&cli);
        assert!(deps.contains(&normalize(&common)));

        fs::remove_dir_all(dir).ok();
    }

    // -----------------------------------------------------------------------
    // Import tallies (coverage)
    // -----------------------------------------------------------------------

    fn tally_for<'a>(tallies: &'a HashMap<PathBuf, ImportTally>, file: &Path) -> &'a ImportTally {
        &tallies[&normalize(file)]
    }

    #[test]
    fn tallies_js_relative_alias_workspace_and_external_imports() {
        let dir = unique_temp_dir("codedna_dg_tally_js");
        let app = dir.join("web/src/app.ts");
        let util = dir.join("web/src/util.ts");
        let pkg = dir.join("packages/ui/package.json");
        write_file(&pkg, r#"{ "name": "@acme/ui" }"#);
        write_file(&util, "export {}");
        write_file(
            &app,
            "import u from './util';\n\
             import again from './util';\n\
             import gone from './gone';\n\
             import b from '@/missing';\n\
             import ui from '@acme/ui/button';\n\
             import React from 'react';\n\
             import x from `./${name}`;\n",
        );
        let files = [app.clone(), util, pkg];
        let (_, tallies) = build_with_tallies(&files);
        let t = tally_for(&tallies, &app);
        assert!(t.readable);
        assert_eq!(t.resolved, 1, "./util counts once");
        assert_eq!(t.external, 1, "only react is external");
        let unresolved: Vec<&str> = t.unresolved.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(unresolved, ["./gone", "@/missing", "@acme/ui/button"]);
        assert_eq!(t.unresolved[0].0, 3, "line of ./gone");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_workspace_package_imports_through_exports() {
        let dir = unique_temp_dir("codedna_dg_workspace");
        let write = |rel: &str, text: &str| {
            let path = dir.join(rel);
            write_file(&path, text);
            path
        };
        let files = vec![
            write(
                "packages/shared/package.json",
                r#"{ "name": "@demo/shared", "exports": "./src/index.ts" }"#,
            ),
            write("packages/shared/src/index.ts", "export {}"),
            write(
                "packages/ui/package.json",
                r#"{ "name": "@demo/ui", "exports": {
                    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
                    "./button": "./src/button.tsx",
                    "./icons/*": { "default": "./dist/icons/*.js" }
                } }"#,
            ),
            write("packages/ui/src/index.ts", "export {}"),
            write("packages/ui/src/button.tsx", "export {}"),
            write("packages/ui/src/icons/star.tsx", "export {}"),
            write(
                "packages/old/package.json",
                r#"{ "name": "old-style", "main": "lib/main.js" }"#,
            ),
            write("packages/old/src/main.ts", "export {}"),
            write("packages/old/src/extra/util.ts", "export {}"),
            write(
                "apps/web/app.ts",
                "import a from '@demo/shared';\n\
                 import b from '@demo/ui';\n\
                 import c from '@demo/ui/button';\n\
                 import d from '@demo/ui/icons/star';\n\
                 import e from 'old-style';\n\
                 import f from 'old-style/extra/util';\n\
                 import g from '@demo/ui/not-exported';\n",
            ),
        ];
        let app = normalize(&dir.join("apps/web/app.ts"));
        let (graph, tallies) = build_with_tallies(&files);
        let mut deps: Vec<String> = graph[&app]
            .iter()
            .map(|p| {
                p.strip_prefix(normalize(&dir))
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        deps.sort();
        assert_eq!(
            deps,
            [
                "packages/old/src/extra/util.ts",
                "packages/old/src/main.ts",
                "packages/shared/src/index.ts",
                "packages/ui/src/button.tsx",
                "packages/ui/src/icons/star.tsx",
                "packages/ui/src/index.ts",
            ]
        );
        let unresolved: Vec<&str> = tallies[&app]
            .unresolved
            .iter()
            .map(|(_, s)| s.as_str())
            .collect();
        assert_eq!(
            unresolved,
            ["@demo/ui/not-exported"],
            "exports hide other paths"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tallies_python_local_package_misses_but_not_third_party() {
        let dir = unique_temp_dir("codedna_dg_tally_py");
        let init = dir.join("shop/__init__.py");
        let models = dir.join("shop/models.py");
        let views = dir.join("shop/views.py");
        write_file(&init, "");
        write_file(&models, "");
        write_file(
            &views,
            "from shop import models\nfrom shop.gone import thing\nimport django\nfrom . import missing\n",
        );
        let files = [init, models, views.clone()];
        let (_, tallies) = build_with_tallies(&files);
        let t = tally_for(&tallies, &views);
        assert_eq!(t.external, 1, "django: {t:?}");
        assert!(t.resolved >= 1, "{t:?}");
        let unresolved: Vec<&str> = t.unresolved.iter().map(|(_, s)| s.as_str()).collect();
        assert!(unresolved.iter().any(|s| s.contains("shop.gone")), "{t:?}");
        assert!(!unresolved.iter().any(|s| s.contains("django")), "{t:?}");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tallies_missing_rust_module() {
        let dir = unique_temp_dir("codedna_dg_tally_rs");
        let main = dir.join("src/main.rs");
        let a = dir.join("src/a.rs");
        write_file(&a, "");
        write_file(&main, "mod a;\nmod b;\nuse serde::Serialize;\n");
        let (_, tallies) = build_with_tallies(&[main.clone(), a]);
        let t = tally_for(&tallies, &main);
        assert_eq!(t.resolved, 1);
        assert_eq!(t.unresolved, vec![(2, "mod b;".to_string())]);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unreadable_file_is_tallied_as_unreadable() {
        let dir = unique_temp_dir("codedna_dg_tally_missing");
        let ghost = dir.join("ghost.ts");
        let (_, tallies) = build_with_tallies(std::slice::from_ref(&ghost));
        assert!(!tally_for(&tallies, &ghost).readable);

        fs::remove_dir_all(dir).ok();
    }
}
