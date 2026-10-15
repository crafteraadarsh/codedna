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

use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
pub fn build_dependency_graph(files: &[PathBuf]) -> DependencyGraph {
    let aliases = Aliases::from_files(files);

    // Each file is parsed independently — no shared mutable state needed.
    // rayon collects directly into a HashMap via the FromParallelIterator impl.
    files
        .par_iter()
        .filter(|file| is_source_file(file))
        .map(|file| {
            let key = normalize(file);
            let deps = parse_dependencies_with(&key, &aliases);
            (key, deps)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript path aliases
// ---------------------------------------------------------------------------

/// Largest `tsconfig.json` / `jsconfig.json` read for path aliases.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Aliases used without any config by Vite, Nuxt and Create React App
/// templates: `@/x` and `~/x` mean `src/x` (or `x` at the package root).
const DEFAULT_ALIASES: &[(&str, &[&str])] = &[("@/*", &["src/*", "*"]), ("~/*", &["src/*", "*"])];

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
}

impl Aliases {
    fn from_files(files: &[PathBuf]) -> Self {
        let mut scopes: BTreeMap<PathBuf, AliasScope> = BTreeMap::new();
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
            if name != "package.json" {
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
        }
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
        None
    }

    fn known_file(&self, base: &Path, target: &str) -> Option<PathBuf> {
        resolve_js_ts_path(base, target).filter(|p| self.known.contains(p))
    }
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
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "py", "rs"];

/// Whether `file` is a source file the dependency graph understands.
fn is_source_file(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SOURCE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Parse dependencies for a single file, with no path aliases.
#[cfg(test)]
fn parse_dependencies(file: &Path) -> Vec<PathBuf> {
    parse_dependencies_with(file, &Aliases::default())
}

/// Parse dependencies for a single file based on its extension.
fn parse_dependencies_with(file: &Path, aliases: &Aliases) -> Vec<PathBuf> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" => parse_js_ts_imports(file, aliases),
        "py" => parse_python_imports(file),
        "rs" => parse_rust_imports(file),
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
fn parse_js_ts_imports(file: &Path, aliases: &Aliases) -> Vec<PathBuf> {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let parent = match file.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        // Collect all quoted string literals on this line that look like
        // relative paths.
        for specifier in extract_quoted_specifiers(trimmed) {
            let resolved = if is_relative(specifier) {
                resolve_js_ts_path(parent, specifier)
            } else {
                aliases.resolve(file, specifier)
            };

            if let Some(resolved) = resolved {
                if !deps.contains(&resolved) {
                    deps.push(resolved);
                }
            }
        }
    }

    deps
}

/// Extract all single- or double-quoted string values from a line that are
/// preceded by `from`, `require`, or `import`.
fn extract_quoted_specifiers(line: &str) -> Vec<&str> {
    let mut results = Vec::new();

    // We scan for from/require/import( patterns then pull the next quoted string.
    let triggers = ["from ", "require(", "import(", "import "];

    for trigger in triggers {
        let mut search = line;
        while let Some(pos) = search.find(trigger) {
            let after = &search[pos + trigger.len()..].trim_start();

            if let Some(specifier) = extract_first_quoted(after) {
                results.push(specifier);
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
    if base.is_file() {
        return Some(normalize(&base));
    }

    // 2. With extension appended
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = append_extension(&base, ext);
        if candidate.is_file() {
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
            if candidate.is_file() {
                return Some(normalize(&candidate));
            }
        }
    }

    // 4. Index file inside directory
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = base.join(format!("index.{ext}"));
        if candidate.is_file() {
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

/// Parse Python imports and return the files they load.
///
/// Patterns matched:
/// - `from .module import ...` and `from ..module import ...`
/// - `from . import module`
/// - `from package.module import ...` and `import package.module`
/// - dotted module paths in string literals, such as Django's
///   `ROOT_URLCONF = "project.urls"` or `include("app.urls")`
///
/// Third-party and stdlib imports find no matching file and are ignored.
fn parse_python_imports(file: &Path) -> Vec<PathBuf> {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let parent = match file.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with('#') {
            continue;
        }

        let found = resolve_python_import(parent, trimmed)
            .into_iter()
            .chain(resolve_python_string_paths(parent, trimmed));
        for resolved in found {
            if !deps.contains(&resolved) {
                deps.push(resolved);
            }
        }
    }

    deps
}

/// Resolve one `from ... import ...` or `import ...` line to files on disk.
fn resolve_python_import(file_parent: &Path, line: &str) -> Vec<PathBuf> {
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
        return resolve_python_module_in(file_parent, module.trim(), &names);
    }

    if let Some(rest) = line.strip_prefix("import ") {
        let mut deps = Vec::new();
        for module in rest.split(',') {
            if let Some(module) = module.split_whitespace().next() {
                deps.extend(resolve_python_module_in(file_parent, module, &[]));
            }
        }
        return deps;
    }

    Vec::new()
}

/// Resolve quoted dotted paths (`"app.views"`, `"app.auth.Backend"`) to the
/// modules they name. Frameworks load these by name: Django settings,
/// URL includes, Celery task names. A trailing attribute is dropped when the
/// full path is not a module.
fn resolve_python_string_paths(file_parent: &Path, line: &str) -> Vec<PathBuf> {
    let mut deps = Vec::new();
    for literal in python_string_literals(line) {
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
                deps.extend(found);
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
    let dot_count = module.chars().take_while(|&c| c == '.').count();
    let dotted = &module[dot_count..];

    if dot_count > 0 {
        return file_parent
            .ancestors()
            .nth(dot_count - 1)
            .and_then(|root| resolve_python_module(root, dotted, names))
            .unwrap_or_default();
    }

    file_parent
        .ancestors()
        .find_map(|root| resolve_python_module(root, dotted, names))
        .unwrap_or_default()
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
        if i + 1 == segments.len() && as_file.is_file() {
            files.push(normalize(&as_file));
            return Some(files);
        }

        dir = dir.join(segment);
        if !dir.is_dir() {
            return None;
        }
        push_if_file(&mut files, &dir.join("__init__.py"));
    }

    for name in names {
        let as_file = dir.join(format!("{name}.py"));
        if as_file.is_file() {
            files.push(normalize(&as_file));
        } else {
            push_if_file(&mut files, &dir.join(name).join("__init__.py"));
        }
    }

    Some(files)
}

/// Push `candidate` onto `files` when it exists.
fn push_if_file(files: &mut Vec<PathBuf>, candidate: &Path) {
    if candidate.is_file() {
        files.push(normalize(candidate));
    }
}

// ---------------------------------------------------------------------------
// Rust parser
// ---------------------------------------------------------------------------

/// Parse Rust module declarations, returning the files they load.
///
/// Patterns matched:
/// - `mod name;` with any visibility (`pub`, `pub(crate)`, ...) and
///   leading attributes such as `#[cfg(test)]`
/// - `#[path = "file.rs"] mod name;`
///
/// A crate root or `mod.rs` loads `name.rs` / `name/mod.rs` next to it;
/// any other file `foo.rs` loads them from `foo/`.
fn parse_rust_imports(file: &Path) -> Vec<PathBuf> {
    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let parent = match file.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let module_dir = rust_module_dir(file, parent);

    let mut deps = Vec::new();
    let mut path_attr: Option<String> = None;

    for line in content.lines() {
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
            let resolved = match path_attr.take() {
                Some(path) => {
                    let candidate = parent.join(path);
                    candidate.is_file().then(|| normalize(&candidate))
                }
                None => resolve_rust_mod(&module_dir, module_name),
            };
            if let Some(resolved) = resolved {
                if !deps.contains(&resolved) {
                    deps.push(resolved);
                }
            }
        } else {
            path_attr = None;
        }
    }

    deps
}

/// Directory holding the files of the modules a Rust file declares.
fn rust_module_dir(file: &Path, parent: &Path) -> PathBuf {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir_name = parent.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let crate_root = matches!(stem, "mod" | "lib" | "main")
        || matches!(dir_name, "bin" | "tests" | "examples" | "benches")
        || (stem == "build" && parent.join("Cargo.toml").is_file());
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
    if as_file.is_file() {
        return Some(normalize(&as_file));
    }

    let as_mod = dir.join(module_name).join("mod.rs");
    if as_mod.is_file() {
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
fn normalize(path: &Path) -> PathBuf {
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
        let graph = build_dependency_graph(&[missing.clone()]);

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
}
