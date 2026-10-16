//! Evidence-based architecture inference.
//!
//! The framework summary in `AnalysisResult::architecture` only says which
//! kinds of technology are present. This module looks at the code itself:
//!
//! 1. Each source file is assigned a layer from its folder names and file
//!    name (`routes/`, `user.controller.ts`, `services/`, `models.py`, …).
//! 2. Import edges between layers are counted from the dependency graph.
//! 3. Confidence depends on how much of that evidence agrees: layers that
//!    import each other top-down (routes → services → data) give high
//!    confidence; folders without supporting imports give medium; no layered
//!    folders at all give low.

use crate::safe_text::sanitize;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// How strongly the evidence supports a conclusion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    #[default]
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// One architectural layer found in the code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Layer name, e.g. `"Routes"`.
    pub name: String,
    /// Number of source files assigned to it.
    pub files: usize,
    /// Up to three folders that hold most of its files.
    pub dirs: Vec<String>,
}

/// Import edges from one layer to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerFlow {
    pub from: String,
    pub to: String,
    pub imports: usize,
}

/// Layers, the imports between them, and how confident the result is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureInsight {
    /// Layers in top-down order (UI, Routes, Middleware, Services, Data).
    pub layers: Vec<Layer>,
    /// Import counts between layers, largest first.
    pub flows: Vec<LayerFlow>,
    pub confidence: Confidence,
    /// Short, human-readable reasons for the result.
    pub evidence: Vec<String>,
}

impl ArchitectureInsight {
    /// `"Routes → Services → Data"`, or `None` when no layers were found.
    pub fn layer_chain(&self) -> Option<String> {
        if self.layers.is_empty() {
            return None;
        }
        Some(
            self.layers
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(" → "),
        )
    }
}

/// Layers in top-down order, with the folder and file names that mark them.
const LAYERS: &[(&str, &[&str])] = &[
    (
        "UI",
        &[
            "components",
            "component",
            "pages",
            "screens",
            "layouts",
            "ui",
            "widgets",
            "hooks",
            "frontend",
            "client",
        ],
    ),
    (
        "Routes",
        &[
            "routes",
            "route",
            "router",
            "routers",
            "controllers",
            "controller",
            "handlers",
            "handler",
            "endpoints",
            "resolvers",
            "resolver",
            "api",
            "views",
            "urls",
        ],
    ),
    (
        "Middleware",
        &[
            "middleware",
            "middlewares",
            "guards",
            "guard",
            "interceptors",
        ],
    ),
    (
        "Services",
        &[
            "services",
            "service",
            "usecases",
            "use_cases",
            "use-cases",
            "domain",
            "jobs",
            "workers",
        ],
    ),
    (
        "Data",
        &[
            "models",
            "model",
            "entities",
            "entity",
            "repositories",
            "repository",
            "repos",
            "db",
            "database",
            "dao",
            "schema",
            "schemas",
            "migrations",
            "prisma",
        ],
    ),
];

/// Extensions of UI files; `views/` and `pages/` only mean UI for these.
const UI_EXTENSIONS: &[&str] = &["tsx", "jsx", "vue", "svelte", "astro"];

/// Most evidence lines kept in the result.
const MAX_EVIDENCE: usize = 6;

/// Infer layers from file locations and the import graph.
///
/// Paths in `graph` may be absolute; `root` is stripped for display.
pub fn infer(
    root: &Path,
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    frameworks: &[String],
    databases: &[String],
) -> ArchitectureInsight {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();

    let mut layer_of: HashMap<&PathBuf, usize> = HashMap::new();
    let mut dir_counts: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); LAYERS.len()];
    for file in graph.keys() {
        let relative = rel(file);
        if let Some(index) = classify(&relative) {
            layer_of.insert(file, index);
            let dir = relative
                .parent()
                .map(|d| sanitize(&d.to_string_lossy().replace('\\', "/")))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| ".".to_string());
            *dir_counts[index].entry(dir).or_default() += 1;
        }
    }

    let mut edges: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (file, deps) in graph {
        let Some(&from) = layer_of.get(file) else {
            continue;
        };
        for dep in deps {
            if let Some(&to) = layer_of.get(dep) {
                if from != to {
                    *edges.entry((from, to)).or_default() += 1;
                }
            }
        }
    }

    let layers: Vec<Layer> = dir_counts
        .iter()
        .enumerate()
        // A single file with no imports to other layers is too weak to
        // count (`git_handler.rs` is not a routing layer).
        .filter(|(index, dirs)| {
            let files: usize = dirs.values().sum();
            files >= 2
                || (files == 1 && edges.keys().any(|(from, to)| from == index || to == index))
        })
        .map(|(index, dirs)| {
            let mut top: Vec<(&String, &usize)> = dirs.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            Layer {
                name: LAYERS[index].0.to_string(),
                files: dirs.values().sum(),
                dirs: top
                    .into_iter()
                    .take(3)
                    .map(|(d, _)| {
                        if d == "." {
                            "./".to_string()
                        } else {
                            crate::analysis::dotted(&format!("{d}/"))
                        }
                    })
                    .collect(),
            }
        })
        .collect();

    let mut flows: Vec<LayerFlow> = edges
        .iter()
        .map(|(&(from, to), &imports)| LayerFlow {
            from: LAYERS[from].0.to_string(),
            to: LAYERS[to].0.to_string(),
            imports,
        })
        .collect();
    flows.sort_by_key(|f| std::cmp::Reverse(f.imports));

    let downward: usize = edges
        .iter()
        .filter(|((from, to), _)| from < to)
        .map(|(_, n)| n)
        .sum();
    let upward: usize = edges
        .iter()
        .filter(|((from, to), _)| from > to)
        .map(|(_, n)| n)
        .sum();

    let confidence = if layers.len() >= 2 && downward >= 2 && downward > upward {
        Confidence::High
    } else if !layers.is_empty() {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    let mut evidence: Vec<String> = Vec::new();
    for layer in &layers {
        evidence.push(format!(
            "{}: {} file{} in {}",
            layer.name,
            layer.files,
            if layer.files == 1 { "" } else { "s" },
            layer.dirs.join(", ")
        ));
    }
    for flow in flows.iter().take(3) {
        evidence.push(format!(
            "{} → {}: {} import{}",
            flow.from,
            flow.to,
            flow.imports,
            if flow.imports == 1 { "" } else { "s" }
        ));
    }
    if upward > 0 {
        evidence.push(format!(
            "{upward} import{} go against the usual direction (lower layer importing a higher one)",
            if upward == 1 { "" } else { "s" }
        ));
    }
    if layers.is_empty() {
        evidence.push("No layered folders (routes, services, models, …) were found".to_string());
        if !frameworks.is_empty() || !databases.is_empty() {
            let mut stack: Vec<&str> = frameworks.iter().map(String::as_str).collect();
            stack.extend(databases.iter().map(String::as_str));
            evidence.push(format!(
                "Summary is based on detected technologies only: {}",
                stack.join(", ")
            ));
        }
    }
    evidence.truncate(MAX_EVIDENCE);

    ArchitectureInsight {
        layers,
        flows,
        confidence,
        evidence,
    }
}

/// Layer index for a relative path, from its file name first, then its
/// closest matching folder. Tests mirror the layout they test, so they are
/// left out.
fn classify(path: &Path) -> Option<usize> {
    if is_test(path) {
        return None;
    }
    let is_ui_file = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| UI_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    match classify_by_name(path)? {
        // File-based routes written as components (`routes/index.tsx`) are UI.
        1 if is_ui_file => Some(0),
        index => Some(index),
    }
}

/// Whether a path is a test file or lives in a test folder.
pub(crate) fn is_test(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains(".test.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_test.go")
        || path.parent().is_some_and(|p| {
            p.iter().filter_map(|c| c.to_str()).any(|c| {
                matches!(
                    c.to_ascii_lowercase().as_str(),
                    "test" | "tests" | "__tests__" | "spec" | "e2e"
                )
            })
        })
}

fn classify_by_name(path: &Path) -> Option<usize> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_ui_file = UI_EXTENSIONS.contains(&ext.as_str());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // `user.controller.ts`, `user_service.py`, `UserRepository.java`.
    let marker = stem
        .rsplit(['.', '_', '-'])
        .next()
        .filter(|m| *m != stem)
        .map(str::to_string);
    for name in marker.iter().chain(std::iter::once(&stem)) {
        if let Some(index) = layer_for_name(name, is_ui_file) {
            return Some(index);
        }
    }

    path.parent()?
        .iter()
        .rev()
        .filter_map(|c| c.to_str())
        .find_map(|dir| layer_for_name(&dir.to_ascii_lowercase(), is_ui_file))
}

fn layer_for_name(name: &str, is_ui_file: bool) -> Option<usize> {
    // Server-rendered `views/` and file-routed `pages/` are routes unless the
    // file is a UI component.
    if matches!(name, "views" | "pages") && is_ui_file {
        return Some(0);
    }
    if name == "pages" {
        return Some(1);
    }
    LAYERS
        .iter()
        .position(|(_, names)| names.contains(&name))
        .filter(|&index| index != 0 || is_ui_file || name != "client")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> HashMap<PathBuf, Vec<PathBuf>> {
        edges
            .iter()
            .map(|(f, deps)| {
                (
                    PathBuf::from("/repo").join(f),
                    deps.iter()
                        .map(|d| PathBuf::from("/repo").join(d))
                        .collect(),
                )
            })
            .collect()
    }

    fn name(path: &str) -> Option<&'static str> {
        classify(Path::new(path)).map(|i| LAYERS[i].0)
    }

    #[test]
    fn classifies_by_folder_and_file_name() {
        assert_eq!(name("src/routes/users.ts"), Some("Routes"));
        assert_eq!(name("src/users/user.controller.ts"), Some("Routes"));
        assert_eq!(name("app/user_service.py"), Some("Services"));
        assert_eq!(name("app/models.py"), Some("Data"));
        assert_eq!(name("src/db/pool.rs"), Some("Data"));
        assert_eq!(name("src/components/Button.tsx"), Some("UI"));
        assert_eq!(name("src/views/Home.vue"), Some("UI"));
        assert_eq!(name("blog/views.py"), Some("Routes"));
        assert_eq!(name("src/utils/strings.ts"), None);
        assert_eq!(name("src/main.rs"), None);
        assert_eq!(name("frontend/src/routes/index.tsx"), Some("UI"));
        assert_eq!(name("tests/unit/models/user.test.js"), None);
        assert_eq!(name("src/services/user.spec.ts"), None);
    }

    #[test]
    fn closest_folder_wins() {
        assert_eq!(name("src/services/models/cache.ts"), Some("Data"));
        assert_eq!(name("src/models/services/cache.ts"), Some("Services"));
    }

    #[test]
    fn top_down_imports_give_high_confidence() {
        let g = graph(&[
            ("src/routes/users.ts", &["src/services/users.ts"]),
            ("src/routes/posts.ts", &["src/services/posts.ts"]),
            ("src/services/users.ts", &["src/models/user.ts"]),
            ("src/services/posts.ts", &["src/models/post.ts"]),
            ("src/models/user.ts", &[]),
            ("src/models/post.ts", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &g, &[], &[]);
        assert_eq!(insight.confidence, Confidence::High);
        assert_eq!(
            insight.layer_chain().as_deref(),
            Some("Routes → Services → Data")
        );
        assert_eq!(insight.layers[0].dirs, ["./src/routes/"]);
        assert!(insight
            .flows
            .iter()
            .any(|f| f.from == "Routes" && f.to == "Services" && f.imports == 2));
    }

    #[test]
    fn folders_without_imports_give_medium_confidence() {
        let g = graph(&[
            ("src/routes/a.ts", &[]),
            ("src/routes/b.ts", &[]),
            ("src/models/c.ts", &[]),
            ("src/models/d.ts", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &g, &[], &[]);
        assert_eq!(insight.confidence, Confidence::Medium);
    }

    #[test]
    fn upward_imports_lower_confidence() {
        let g = graph(&[
            ("src/models/a.ts", &["src/routes/x.ts", "src/routes/y.ts"]),
            ("src/models/b.ts", &["src/routes/x.ts"]),
            ("src/routes/x.ts", &["src/models/b.ts"]),
            ("src/routes/y.ts", &["src/models/a.ts"]),
        ]);
        let insight = infer(Path::new("/repo"), &g, &[], &[]);
        assert_eq!(insight.confidence, Confidence::Medium);
        assert!(insight.evidence.iter().any(|e| e.contains("against")));
    }

    #[test]
    fn lone_file_without_imports_is_not_a_layer() {
        let g = graph(&[("src/git_handler.rs", &[]), ("src/main.rs", &[])]);
        let insight = infer(Path::new("/repo"), &g, &[], &[]);
        assert!(insight.layers.is_empty());
        assert_eq!(insight.confidence, Confidence::Low);
    }

    #[test]
    fn no_layers_falls_back_to_technologies() {
        let g = graph(&[
            ("src/main.rs", &["src/scanner.rs"]),
            ("src/scanner.rs", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &g, &["Axum".to_string()], &[]);
        assert_eq!(insight.confidence, Confidence::Low);
        assert!(insight.layer_chain().is_none());
        assert!(insight.evidence.iter().any(|e| e.contains("Axum")));
    }
}
