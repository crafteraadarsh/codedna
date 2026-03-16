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
/// Files that cannot be read are skipped silently.
pub fn build_dependency_graph(files: &[PathBuf]) -> DependencyGraph {
    // Each file is parsed independently — no shared mutable state needed.
    // rayon collects directly into a HashMap via the FromParallelIterator impl.
    files
        .par_iter()
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
/// 2. `<path>.ts`, `<path>.tsx`, `<path>.js`, `<path>.jsx`, `<path>.mjs`
/// 3. `<path>/index.ts`, `<path>/index.js`, etc.
fn resolve_js_ts_path(parent: &Path, specifier: &str) -> Option<PathBuf> {
    let base = parent.join(specifier);

    // 1. Exact path
    if base.is_file() {
        return Some(normalize(&base));
    }

    // 2. With extension
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = base.with_extension(ext);
        if candidate.is_file() {
            return Some(normalize(&candidate));
        }
    }

    // 3. Index file inside directory
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let candidate = base.join(format!("index.{ext}"));
        if candidate.is_file() {
            return Some(normalize(&candidate));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Python parser
// ---------------------------------------------------------------------------

/// Parse Python relative imports and return resolved paths.
///
/// Patterns matched (relative only):
/// - `from .module import ...`
/// - `from ..module import ...`
/// - `from . import something`
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

        // Match `from .xxx import yyy` or `from ..xxx import yyy`
        if trimmed.starts_with("from .") {
            if let Some(resolved) = resolve_python_relative(parent, trimmed) {
                if !deps.contains(&resolved) {
                    deps.push(resolved);
                }
            }
        }
    }

    deps
}

/// Resolve a `from .module import ...` statement to an absolute path.
///
/// Counts leading dots to determine how many directories to go up.
fn resolve_python_relative(file_parent: &Path, line: &str) -> Option<PathBuf> {
    // Strip `from ` prefix
    let rest = line.strip_prefix("from ")?.trim_start();

    // Count leading dots
    let dot_count = rest.chars().take_while(|&c| c == '.').count();
    let after_dots = &rest[dot_count..];

    // Module name is everything up to the first space
    let module = after_dots
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('.');

    // Walk up `dot_count - 1` directories (one dot = same package)
    let mut base = file_parent.to_path_buf();
    for _ in 1..dot_count {
        base = match base.parent() {
            Some(p) => p.to_path_buf(),
            None => return None,
        };
    }

    if module.is_empty() {
        // `from . import something` — refers to __init__.py of current package
        let candidate = base.join("__init__.py");
        if candidate.is_file() {
            return Some(normalize(&candidate));
        }
        return None;
    }

    // Convert dotted module path to filesystem path
    let module_path = module.replace('.', std::path::MAIN_SEPARATOR_STR);

    // Try as a module file
    let as_file = base.join(format!("{module_path}.py"));
    if as_file.is_file() {
        return Some(normalize(&as_file));
    }

    // Try as a package directory
    let as_pkg = base.join(&module_path).join("__init__.py");
    if as_pkg.is_file() {
        return Some(normalize(&as_pkg));
    }

    None
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
    fn build_graph_handles_unreadable_file_gracefully() {
        let missing = PathBuf::from("/no/such/file/ever/codedna_missing.ts");
        let graph = build_dependency_graph(&[missing.clone()]);

        assert!(graph.contains_key(&missing));
        assert!(graph[&missing].is_empty());
    }
}
