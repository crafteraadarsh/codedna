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
use std::collections::HashMap;
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
    // Each file is parsed independently — no shared mutable state needed.
    // rayon collects directly into a HashMap via the FromParallelIterator impl.
    files
        .par_iter()
        .filter(|file| is_source_file(file))
        .map(|file| {
            let key = normalize(file);
            let deps = parse_dependencies(&key);
            (key, deps)
        })
        .collect()
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

/// Parse dependencies for a single file based on its extension.
fn parse_dependencies(file: &Path) -> Vec<PathBuf> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" => parse_js_ts_imports(file),
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
/// - `import('./path')`
fn parse_js_ts_imports(file: &Path) -> Vec<PathBuf> {
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
            if !is_relative(specifier) {
                continue;
            }

            if let Some(resolved) = resolve_js_ts_path(parent, specifier) {
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
    let triggers = ["from ", "require(", "import("];

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

        for resolved in resolve_python_import(parent, trimmed) {
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

/// Parse Rust module declarations and `use` statements, returning resolved paths.
///
/// Patterns matched:
/// - `mod module_name;`          → resolves to `module_name.rs` or
///   `module_name/mod.rs`
/// - `use crate::a::b;`          → resolves relative to src/
/// - `use super::module;`        → resolves relative to parent
fn parse_rust_imports(file: &Path) -> Vec<PathBuf> {
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

        // `mod module_name;`
        if let Some(module_name) = parse_rust_mod_decl(trimmed) {
            if let Some(resolved) = resolve_rust_mod(parent, module_name) {
                if !deps.contains(&resolved) {
                    deps.push(resolved);
                }
            }
        }
    }

    deps
}

/// Extract module name from `mod name;` declarations.
///
/// Returns `None` for `mod name { ... }` (inline modules).
fn parse_rust_mod_decl(line: &str) -> Option<&str> {
    let line = line.trim_start_matches("pub").trim_start();
    let rest = line.strip_prefix("mod ")?.trim();

    // Only handle `mod name;` not `mod name {`
    if rest.ends_with(';') {
        Some(rest.trim_end_matches(';').trim())
    } else {
        None
    }
}

/// Resolve a Rust `mod name;` declaration to a file on disk.
///
/// Tries:
/// 1. `<parent>/<name>.rs`
/// 2. `<parent>/<name>/mod.rs`
fn resolve_rust_mod(parent: &Path, module_name: &str) -> Option<PathBuf> {
    let as_file = parent.join(format!("{module_name}.rs"));
    if as_file.is_file() {
        return Some(normalize(&as_file));
    }

    let as_mod = parent.join(module_name).join("mod.rs");
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
}
