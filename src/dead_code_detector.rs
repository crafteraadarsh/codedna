//! Dead code detection module.
//!
//! Consumes a dependency graph and identifies files that are never reachable
//! from any known entry point via BFS traversal.
//!
//! A file is considered dead code when:
//! - It is not an entry point
//! - It is not reachable (directly or transitively) from any entry point
//!
//! Entry points are files whose names match well-known patterns:
//! `main.rs`, `index.ts`, `index.js`, `app.py`, `manage.py`, etc.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Entry point patterns
// ---------------------------------------------------------------------------

/// File names that are always treated as entry points regardless of location.
const ENTRY_POINT_NAMES: &[&str] = &[
    // Rust
    "main.rs",
    "lib.rs",
    "build.rs",
    // JavaScript / TypeScript
    "index.ts",
    "index.tsx",
    "index.js",
    "index.jsx",
    "index.mjs",
    "server.ts",
    "server.js",
    "app.ts",
    "app.js",
    "main.ts",
    "main.js",
    "vite.config.ts",
    "vite.config.js",
    "next.config.ts",
    "next.config.js",
    "tailwind.config.ts",
    "tailwind.config.js",
    "webpack.config.js",
    "rollup.config.js",
    "jest.config.ts",
    "jest.config.js",
    "vitest.config.ts",
    "vitest.config.js",
    // Python
    "app.py",
    "main.py",
    "run.py",
    "manage.py",
    "wsgi.py",
    "asgi.py",
    "__main__.py",
    // Go
    "main.go",
    // General CLI / config entrypoints
    "cli.ts",
    "cli.js",
    "cli.rs",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect dead (unreachable) files from a dependency graph.
///
/// # Algorithm
///
/// 1. Collect all files present as keys in the graph.
/// 2. Identify entry points by filename.
/// 3. BFS from all entry points, marking every transitively reachable file.
/// 4. Files in the graph that were never marked are dead code.
///
/// Returns a sorted `Vec<PathBuf>` of unreachable files.
pub fn detect_dead_code(graph: &HashMap<PathBuf, Vec<PathBuf>>) -> Vec<PathBuf> {
    if graph.is_empty() {
        return Vec::new();
    }

    let all_files: HashSet<&PathBuf> = graph.keys().collect();
    let entry_points: Vec<&PathBuf> = all_files
        .iter()
        .copied()
        .filter(|f| is_entry_point(f))
        .collect();

    let reachable = bfs_reachable(graph, &entry_points);

    let mut dead: Vec<PathBuf> = all_files
        .into_iter()
        .filter(|f| !reachable.contains(*f))
        .cloned()
        .collect();

    dead.sort_unstable();
    dead
}

/// Return the set of all files reachable from the given entry points via BFS.
fn bfs_reachable<'a>(
    graph: &'a HashMap<PathBuf, Vec<PathBuf>>,
    entry_points: &[&'a PathBuf],
) -> HashSet<&'a PathBuf> {
    let mut visited: HashSet<&PathBuf> = HashSet::new();
    let mut queue: VecDeque<&PathBuf> = VecDeque::new();

    for &entry in entry_points {
        if visited.insert(entry) {
            queue.push_back(entry);
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(deps) = graph.get(current) {
            for dep in deps {
                // Only traverse files that are actually nodes in the graph.
                if graph.contains_key(dep) && visited.insert(dep) {
                    queue.push_back(dep);
                }
            }
        }
    }

    visited
}

/// Return `true` if the file's name matches a known entry point pattern.
pub fn is_entry_point(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| ENTRY_POINT_NAMES.contains(&name))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// Helper: build a graph from (file, deps) pairs.
    fn make_graph(edges: &[(&str, &[&str])]) -> HashMap<PathBuf, Vec<PathBuf>> {
        edges
            .iter()
            .map(|(file, deps)| (p(file), deps.iter().map(|d| p(d)).collect()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // is_entry_point
    // -----------------------------------------------------------------------

    #[test]
    fn recognises_rust_entry_points() {
        assert!(is_entry_point(&p("src/main.rs")));
        assert!(is_entry_point(&p("src/lib.rs")));
        assert!(is_entry_point(&p("build.rs")));
    }

    #[test]
    fn recognises_ts_js_entry_points() {
        assert!(is_entry_point(&p("src/index.ts")));
        assert!(is_entry_point(&p("src/index.tsx")));
        assert!(is_entry_point(&p("src/index.js")));
        assert!(is_entry_point(&p("src/server.ts")));
        assert!(is_entry_point(&p("app.ts")));
        assert!(is_entry_point(&p("app.js")));
        assert!(is_entry_point(&p("main.ts")));
    }

    #[test]
    fn recognises_python_entry_points() {
        assert!(is_entry_point(&p("app.py")));
        assert!(is_entry_point(&p("main.py")));
        assert!(is_entry_point(&p("manage.py")));
        assert!(is_entry_point(&p("wsgi.py")));
        assert!(is_entry_point(&p("asgi.py")));
        assert!(is_entry_point(&p("__main__.py")));
    }

    #[test]
    fn recognises_config_entry_points() {
        assert!(is_entry_point(&p("vite.config.ts")));
        assert!(is_entry_point(&p("next.config.js")));
        assert!(is_entry_point(&p("jest.config.ts")));
        assert!(is_entry_point(&p("vitest.config.ts")));
    }

    #[test]
    fn does_not_flag_regular_files_as_entry_points() {
        assert!(!is_entry_point(&p("src/utils.ts")));
        assert!(!is_entry_point(&p("src/helpers.rs")));
        assert!(!is_entry_point(&p("models/user.py")));
        assert!(!is_entry_point(&p("components/Button.tsx")));
    }

    // -----------------------------------------------------------------------
    // detect_dead_code — basic cases
    // -----------------------------------------------------------------------

    #[test]
    fn returns_empty_for_empty_graph() {
        let graph = make_graph(&[]);
        assert!(detect_dead_code(&graph).is_empty());
    }

    #[test]
    fn returns_empty_when_all_files_are_reachable() {
        // index.ts → routes.ts → controller.ts
        let graph = make_graph(&[
            ("src/index.ts", &["src/routes.ts"]),
            ("src/routes.ts", &["src/controller.ts"]),
            ("src/controller.ts", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        assert!(dead.is_empty(), "expected no dead code, got: {dead:?}");
    }

    #[test]
    fn detects_unreachable_file() {
        let graph = make_graph(&[
            ("src/index.ts", &["src/routes.ts"]),
            ("src/routes.ts", &[]),
            // This file is never imported
            ("src/utils/oldHelper.ts", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        assert_eq!(dead, vec![p("src/utils/oldHelper.ts")]);
    }

    #[test]
    fn detects_multiple_unreachable_files() {
        let graph = make_graph(&[
            ("src/index.ts", &["src/api.ts"]),
            ("src/api.ts", &[]),
            ("src/legacy/oldAuth.ts", &[]),
            ("src/legacy/deprecatedUI.tsx", &[]),
        ]);

        let mut dead = detect_dead_code(&graph);
        dead.sort_unstable();

        assert_eq!(
            dead,
            vec![p("src/legacy/deprecatedUI.tsx"), p("src/legacy/oldAuth.ts"),]
        );
    }

    #[test]
    fn transitively_reachable_files_are_not_dead() {
        // entry → a → b → c  — all should be alive
        let graph = make_graph(&[
            ("src/main.rs", &["src/scanner.rs"]),
            ("src/scanner.rs", &["src/utils.rs"]),
            ("src/utils.rs", &["src/types.rs"]),
            ("src/types.rs", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        assert!(dead.is_empty(), "expected no dead code, got: {dead:?}");
    }

    #[test]
    fn handles_graph_with_no_entry_point() {
        // No file matches an entry point name → everything is dead
        let graph = make_graph(&[("src/helper.ts", &["src/utils.ts"]), ("src/utils.ts", &[])]);

        let mut dead = detect_dead_code(&graph);
        dead.sort_unstable();

        assert_eq!(dead, vec![p("src/helper.ts"), p("src/utils.ts")]);
    }

    #[test]
    fn handles_multiple_entry_points() {
        // Both index.ts and server.ts are entry points
        let graph = make_graph(&[
            ("src/index.ts", &["src/ui.ts"]),
            ("src/server.js", &["src/api.ts"]),
            ("src/ui.ts", &[]),
            ("src/api.ts", &[]),
            ("src/unused.ts", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        assert_eq!(dead, vec![p("src/unused.ts")]);
    }

    #[test]
    fn handles_cyclic_dependencies_without_infinite_loop() {
        // a → b → c → a (cycle), all reachable from entry
        let graph = make_graph(&[
            ("src/index.ts", &["src/a.ts"]),
            ("src/a.ts", &["src/b.ts"]),
            ("src/b.ts", &["src/c.ts"]),
            ("src/c.ts", &["src/a.ts"]), // cycle back to a
        ]);

        let dead = detect_dead_code(&graph);
        assert!(
            dead.is_empty(),
            "expected no dead code in a cycle, got: {dead:?}"
        );
    }

    #[test]
    fn handles_isolated_cycle_as_dead_code() {
        // entry → live_file
        // orphan_a ↔ orphan_b  (cycle, but never reachable from entry)
        let graph = make_graph(&[
            ("src/index.ts", &["src/live.ts"]),
            ("src/live.ts", &[]),
            ("src/orphan_a.ts", &["src/orphan_b.ts"]),
            ("src/orphan_b.ts", &["src/orphan_a.ts"]),
        ]);

        let mut dead = detect_dead_code(&graph);
        dead.sort_unstable();

        assert_eq!(dead, vec![p("src/orphan_a.ts"), p("src/orphan_b.ts")]);
    }

    #[test]
    fn python_entry_point_keeps_its_imports_alive() {
        let graph = make_graph(&[
            ("app.py", &["models/user.py"]),
            ("models/user.py", &["models/base.py"]),
            ("models/base.py", &[]),
            ("utils/stale.py", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        assert_eq!(dead, vec![p("utils/stale.py")]);
    }

    #[test]
    fn output_is_sorted() {
        let graph = make_graph(&[
            ("src/index.ts", &[]),
            ("src/z_unused.ts", &[]),
            ("src/a_unused.ts", &[]),
            ("src/m_unused.ts", &[]),
        ]);

        let dead = detect_dead_code(&graph);
        let mut expected = dead.clone();
        expected.sort_unstable();

        assert_eq!(dead, expected, "output should be sorted");
    }
}
