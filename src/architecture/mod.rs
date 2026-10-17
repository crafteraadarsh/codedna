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

mod package;
mod roles;

use crate::cache::Cache;
use crate::safe_text::sanitize;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub use package::Purpose;
pub(crate) use roles::is_test;
pub use roles::SETTINGS_FILE;
use roles::*;

/// How strongly the evidence supports a conclusion.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
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

/// A line in the repository that backs a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Location {
    /// Path relative to the repository root.
    pub file: String,
    /// 1-based line number.
    pub line: usize,
    /// The line, trimmed and shortened.
    pub text: String,
}

/// Longest line text kept in a [`Location`].
const MAX_LINE_TEXT: usize = 80;

impl Location {
    /// `line` of `path`, read from disk; the text is empty if it cannot be read.
    pub(crate) fn read(root: &Path, path: &Path, line: usize) -> Self {
        let text = std::fs::read(path)
            .ok()
            .and_then(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .nth(line.saturating_sub(1))
                    .map(str::to_string)
            })
            .unwrap_or_default();
        Self::new(root, path, line, &text)
    }

    pub(crate) fn new(root: &Path, path: &Path, line: usize, text: &str) -> Self {
        Location {
            file: display_path(root, path),
            line,
            text: short_line(text),
        }
    }
}

/// `root`-relative path with forward slashes, safe to print.
fn display_path(root: &Path, path: &Path) -> String {
    sanitize(
        &path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

/// One line of source, trimmed, shortened and safe to print.
fn short_line(text: &str) -> String {
    let trimmed = text.trim();
    let mut line: String = sanitize(trimmed).chars().take(MAX_LINE_TEXT).collect();
    if trimmed.chars().count() > MAX_LINE_TEXT {
        line.push('…');
    }
    line
}

/// What a file's code shows about its layer, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CodeSign {
    /// 1-based line of the match; 0 when the whole file is the sign
    /// (`.vue`, `.svelte`).
    pub line: usize,
    /// What matched: a route (`GET /users`) or the text found (`@Entity(`).
    pub sign: String,
}

/// Why one file is in its layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Placement {
    /// Path relative to the repository root.
    pub file: String,
    /// `settings`, `name`, `code` or `imports`.
    pub placed_by: String,
    /// What placed it: the folder or file name, the settings prefix, the
    /// code sign, or the files it sits between.
    pub matched: String,
    /// The code that shows the layer, when it agrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeSign>,
    /// The folder or file name the code overruled and the layer it
    /// suggested (`services/ (Services)`), when they disagreed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_said: Option<String>,
    /// Other layers the file's code also shows (routes that define a model).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<AlsoShown>,
}

/// Another role a file's code shows besides the layer it is placed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AlsoShown {
    pub layer: String,
    pub code: CodeSign,
}

impl Placement {
    /// `src/routes/user.ts:14  GET /users` — the file and what shows its layer.
    pub fn describe(&self) -> String {
        match &self.code {
            Some(code) => {
                let at = if code.line > 0 {
                    format!("{}:{}", self.file, code.line)
                } else {
                    self.file.clone()
                };
                if self.placed_by == "code" {
                    match &self.name_said {
                        Some(name) => format!("{at}  {}; not {name}", code.sign),
                        None => format!("{at}  {}", code.sign),
                    }
                } else {
                    format!("{at}  {}; {}", self.matched, code.sign)
                }
            }
            None if self.placed_by == "settings" => {
                format!("{}  {SETTINGS_FILE}: {}", self.file, self.matched)
            }
            None => format!("{}  {}", self.file, self.matched),
        }
    }
}

/// One import from a file in one layer to a file in another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
}

/// One architectural layer found in the code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Layer {
    /// Layer name, e.g. `"Routes"`.
    pub name: String,
    /// Number of source files assigned to it.
    pub files: usize,
    /// Up to three folders that hold most of its files.
    pub dirs: Vec<String>,
    /// Files whose code shows the layer (route handlers, models, …).
    #[serde(default)]
    pub confirmed_by_code: usize,
    /// Files placed only by what imports them and what they import.
    #[serde(default)]
    pub placed_by_imports: usize,
    /// A few of its files and why each is here, code-backed ones first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Placement>,
    /// How many of its files also show another layer in their code, by layer.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub also_shown: BTreeMap<String, usize>,
}

/// One package (a folder with a manifest) and what it is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PackageRole {
    /// Folder relative to the repository root; `.` for the root.
    pub path: String,
    pub purpose: Purpose,
    /// What decided the purpose (`package.json depends on express`).
    pub signals: Vec<String>,
    /// Source files in the package, tests left out.
    pub files: usize,
    /// Files per layer its names or code suggest, whether or not they were
    /// counted as application layers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layers: BTreeMap<String, usize>,
}

/// Import edges from one layer to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayerFlow {
    pub from: String,
    pub to: String,
    pub imports: usize,
    /// A few of the imports, by path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ImportEdge>,
}

/// Layers, the imports between them, and how confident the result is.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ArchitectureInsight {
    /// Layers in top-down order (UI, Routes, Middleware, Services, Data).
    pub layers: Vec<Layer>,
    /// Import counts between layers, largest first.
    pub flows: Vec<LayerFlow>,
    pub confidence: Confidence,
    /// Short, human-readable reasons for the result.
    pub evidence: Vec<String>,
    /// Packages and their purpose, largest first (at most 20).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageRole>,
    /// Readings the layers leave out: library folders that look like layers,
    /// project templates, code in languages that are not read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub competing: Vec<String>,
    /// Why the architecture is unknown, when most source is not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<String>,
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

/// Layer names, top-down.
pub const LAYER_NAMES: [&str; 5] = ["UI", "Routes", "Middleware", "Services", "Data"];

/// Most evidence lines kept in the result.
const MAX_EVIDENCE: usize = 8;

/// Most example files kept per layer, and imports per flow.
const MAX_EXAMPLES: usize = 3;

/// Most packages listed.
const MAX_PACKAGES: usize = 20;

/// Most competing readings listed.
const MAX_COMPETING: usize = 6;

/// `UI (3 files), Routes (1 file)` for per-layer counts.
fn layer_counts(counts: &[usize; 5]) -> String {
    counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(i, n)| {
            format!(
                "{} ({n} file{})",
                LAYERS[i].0,
                if *n == 1 { "" } else { "s" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The layer name of every file placed in one, keyed by the paths in
/// `graph` and `files`.
pub fn layer_by_file(
    root: &Path,
    files: &[PathBuf],
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
) -> HashMap<PathBuf, &'static str> {
    place(root, files, graph, &Cache::off())
        .layer_of
        .into_iter()
        .map(|(file, (index, _, _))| (file.clone(), LAYERS[index].0))
        .collect()
}

/// Infer layers from file locations, file contents and the import graph.
///
/// Paths in `graph` may be absolute; `root` is stripped for display.
pub fn infer(
    root: &Path,
    files: &[PathBuf],
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    frameworks: &[String],
    databases: &[String],
    cache: &Cache,
) -> ArchitectureInsight {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
    let Placed {
        by_code,
        mut layer_of,
        name_said,
        packages,
        package_files,
        package_layers,
        not_an_app,
        template,
        template_used,
    } = place(root, files, graph, cache);

    // Packages, largest first, then listed by path.
    let mut package_roles: Vec<PackageRole> = packages
        .list
        .iter()
        .enumerate()
        .map(|(i, p)| PackageRole {
            path: sanitize(&p.display()),
            purpose: p.purpose,
            signals: p.signals.iter().map(|s| sanitize(s)).collect(),
            files: package_files[i],
            layers: package_layers[i]
                .iter()
                .enumerate()
                .filter(|(_, n)| **n > 0)
                .map(|(l, n)| (LAYERS[l].0.to_string(), *n))
                .collect(),
        })
        .collect();
    package_roles.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.path.cmp(&b.path)));
    package_roles.truncate(MAX_PACKAGES);
    package_roles.sort_by(|a, b| a.path.cmp(&b.path));

    let mut competing: Vec<String> = Vec::new();
    let mut left_out: Vec<(&usize, &[usize; 5])> = not_an_app.iter().collect();
    left_out.sort_by_key(|(_, counts)| std::cmp::Reverse(counts.iter().sum::<usize>()));
    for (&index, counts) in left_out {
        let p = &packages.list[index];
        competing.push(format!(
            "{} is a {} ({}): names or code suggest {}; not counted as application layers",
            sanitize(&p.display()),
            p.purpose.as_str(),
            sanitize(&p.signals.join("; ")),
            layer_counts(counts)
        ));
    }
    if template.iter().sum::<usize>() > 0 {
        competing.push(format!(
            "Project templates suggest {}; they describe the projects the templates generate, not this repository",
            layer_counts(&template)
        ));
    }

    // Where most source is in languages that are not read, the layers of
    // the rest are not the architecture.
    let unknown = package::unread_majority(root, files).map(|(language, unread, read)| {
        format!(
            "Most source files are in languages CodeDna does not read ({language}: {unread} files vs {read} supported); the architecture is unknown"
        )
    });
    if unknown.is_some() {
        let mut counts = [0usize; 5];
        for (index, _, _) in layer_of.values() {
            counts[*index] += 1;
        }
        if counts.iter().sum::<usize>() > 0 {
            competing.insert(
                0,
                format!(
                    "Supported code alone shows {}; not reported as the architecture",
                    layer_counts(&counts)
                ),
            );
        }
        layer_of.clear();
    }
    competing.truncate(MAX_COMPETING);

    let mut dir_counts: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); LAYERS.len()];
    let mut confirmed = vec![0usize; LAYERS.len()];
    let mut placed = vec![0usize; LAYERS.len()];
    let mut from_settings = 0usize;
    let mut examples: Vec<Vec<Placement>> = vec![Vec::new(); LAYERS.len()];
    let mut also_shown: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); LAYERS.len()];
    for (file, (index, source, matched)) in &layer_of {
        let (index, source) = (*index, *source);
        let relative = rel(file);
        let dir = relative
            .parent()
            .map(|d| sanitize(&d.to_string_lossy().replace('\\', "/")))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        *dir_counts[index].entry(dir).or_default() += 1;
        let roles = by_code.get(file).map(Vec::as_slice).unwrap_or_default();
        let code = roles
            .iter()
            .find(|(code_index, _)| *code_index == index)
            .map(|(_, sign)| sign.clone());
        if code.is_some() {
            confirmed[index] += 1;
        }
        // Request handlers share the middleware signature, so routes do not
        // also count as middleware.
        let also: Vec<AlsoShown> = roles
            .iter()
            .filter(|(i, _)| *i != index && !(index == 1 && *i == 2))
            .map(|(i, sign)| AlsoShown {
                layer: LAYERS[*i].0.to_string(),
                code: sign.clone(),
            })
            .collect();
        for role in &also {
            *also_shown[index].entry(role.layer.clone()).or_default() += 1;
        }
        examples[index].push(Placement {
            file: display_path(root, file),
            placed_by: source.as_str().to_string(),
            matched: matched.clone(),
            code,
            name_said: name_said.get(file).cloned(),
            also,
        });
        match source {
            Source::Imports => placed[index] += 1,
            Source::Settings => from_settings += 1,
            _ => {}
        }
    }
    // Code-backed examples first, then by path, so output is stable.
    for list in &mut examples {
        list.sort_by(|a, b| {
            b.code
                .is_some()
                .cmp(&a.code.is_some())
                .then_with(|| a.file.cmp(&b.file))
        });
        list.truncate(MAX_EXAMPLES);
    }
    let layer_of: HashMap<&PathBuf, usize> =
        layer_of.into_iter().map(|(f, (i, _, _))| (f, i)).collect();

    let mut edges: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut edge_examples: BTreeMap<(usize, usize), Vec<ImportEdge>> = BTreeMap::new();
    for (file, deps) in graph {
        let Some(&from) = layer_of.get(file) else {
            continue;
        };
        for dep in deps {
            if let Some(&to) = layer_of.get(dep) {
                if from != to {
                    *edges.entry((from, to)).or_default() += 1;
                    edge_examples
                        .entry((from, to))
                        .or_default()
                        .push(ImportEdge {
                            from: display_path(root, file),
                            to: display_path(root, dep),
                        });
                }
            }
        }
    }
    for list in edge_examples.values_mut() {
        list.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.to.cmp(&b.to)));
        list.truncate(MAX_EXAMPLES);
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
                confirmed_by_code: confirmed[index],
                placed_by_imports: placed[index],
                examples: std::mem::take(&mut examples[index]),
                also_shown: std::mem::take(&mut also_shown[index]),
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
            examples: edge_examples.remove(&(from, to)).unwrap_or_default(),
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

    // Code that agrees with the layer names counts as much as imports do.
    // Imports alone count when they mostly run one way: a few lower layers
    // importing higher ones is normal, as many as go down is not layering.
    let code_backed =
        layers.len() >= 2 && layers.iter().all(|l| l.confirmed_by_code > 0) && upward <= downward;
    let one_way = layers.len() >= 2 && downward >= 2 && downward >= 3 * upward;
    let confidence = if unknown.is_some() {
        Confidence::Low
    } else if one_way || code_backed {
        Confidence::High
    } else if !layers.is_empty() {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    let mut evidence: Vec<String> = Vec::new();
    for layer in &layers {
        let mut line = format!(
            "{}: {} file{} in {}",
            layer.name,
            layer.files,
            if layer.files == 1 { "" } else { "s" },
            layer.dirs.join(", ")
        );
        if layer.confirmed_by_code > 0 {
            line.push_str(&format!(
                " ({} confirmed by code: {})",
                layer.confirmed_by_code,
                LAYER_SIGNS[LAYERS
                    .iter()
                    .position(|(n, _)| *n == layer.name)
                    .unwrap_or(0)]
            ));
        }
        evidence.push(line);
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
    let placed_total: usize = placed.iter().sum();
    if placed_total > 0 {
        evidence.push(format!(
            "{placed_total} file{} placed in Services because routes import {} and {} imports data",
            if placed_total == 1 { "" } else { "s" },
            if placed_total == 1 { "it" } else { "them" },
            if placed_total == 1 { "it" } else { "each" },
        ));
    }
    if from_settings > 0 {
        evidence.insert(
            0,
            format!(
                "{from_settings} file{} placed by [layers] in {SETTINGS_FILE}",
                if from_settings == 1 { "" } else { "s" }
            ),
        );
    }
    if upward > 0 {
        evidence.push(format!(
            "{upward} import{} go against the usual direction (lower layer importing a higher one)",
            if upward == 1 { "" } else { "s" }
        ));
    }
    if template_used {
        evidence.insert(
            0,
            "Only project templates show layers; they describe the project the template generates"
                .to_string(),
        );
    }
    if let Some(unknown) = &unknown {
        evidence.insert(0, unknown.clone());
    } else if layers.is_empty() {
        evidence.push(
            "No layers found from folder names or code (routes, services, models, …)".to_string(),
        );
        if !frameworks.is_empty() || !databases.is_empty() {
            let mut stack: Vec<&str> = frameworks.iter().map(String::as_str).collect();
            stack.extend(databases.iter().map(String::as_str));
            evidence.push(format!(
                "Summary is based on detected technologies only: {}",
                stack.join(", ")
            ));
        }
    }
    let mut purposes: BTreeMap<Purpose, usize> = BTreeMap::new();
    for p in &packages.list {
        *purposes.entry(p.purpose).or_default() += 1;
    }
    if packages.list.iter().any(|p| p.purpose != Purpose::Unknown) {
        let list: Vec<String> = purposes
            .iter()
            .filter(|(p, _)| **p != Purpose::Unknown)
            .map(|(p, n)| format!("{n} {}", p.as_str()))
            .collect();
        evidence.push(format!("Package purposes: {}", list.join(", ")));
    }
    evidence.truncate(MAX_EVIDENCE);

    ArchitectureInsight {
        layers,
        flows,
        confidence,
        evidence,
        packages: package_roles,
        competing,
        unknown,
    }
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
        classify(Path::new(path)).map(|(i, _)| LAYERS[i].0)
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        assert_eq!(insight.confidence, Confidence::Medium);
        assert!(insight.evidence.iter().any(|e| e.contains("against")));
    }

    #[test]
    fn imports_both_ways_are_not_layering() {
        // Three imports down and two up, like a library's modules calling
        // each other: the names match layers, the imports don't.
        let g = graph(&[
            ("pkg/api.py", &["pkg/models.py", "pkg/services/s.py"]),
            ("pkg/services/s.py", &["pkg/api.py", "pkg/models.py"]),
            ("pkg/models.py", &["pkg/api.py"]),
            ("pkg/services/t.py", &[]),
            ("pkg/models/u.py", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        assert!(insight.layers.len() >= 2);
        assert_eq!(insight.confidence, Confidence::Medium);
    }

    #[test]
    fn sample_apps_count_only_without_other_layers() {
        let g = graph(&[
            ("examples/blog/routes/a.js", &["examples/blog/models/a.js"]),
            ("examples/blog/routes/b.js", &["examples/blog/models/b.js"]),
            ("examples/blog/models/a.js", &[]),
            ("examples/blog/models/b.js", &[]),
            ("lib/middleware/query.js", &[]),
            ("lib/middleware/init.js", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        assert_eq!(insight.layer_chain().as_deref(), Some("Middleware"));
        assert_eq!(insight.confidence, Confidence::Medium);

        let only_samples: HashMap<PathBuf, Vec<PathBuf>> = g
            .into_iter()
            .filter(|(f, _)| f.starts_with("/repo/examples"))
            .collect();
        let insight = infer(
            Path::new("/repo"),
            &[],
            &only_samples,
            &[],
            &[],
            &Cache::off(),
        );
        assert_eq!(insight.layer_chain().as_deref(), Some("Routes → Data"));
        assert_eq!(insight.confidence, Confidence::High);
    }

    #[test]
    fn lone_file_without_imports_is_not_a_layer() {
        let g = graph(&[("src/git_handler.rs", &[]), ("src/main.rs", &[])]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        assert!(insight.layers.is_empty());
        assert_eq!(insight.confidence, Confidence::Low);
    }

    #[test]
    fn no_layers_falls_back_to_technologies() {
        let g = graph(&[
            ("src/main.rs", &["src/scanner.rs"]),
            ("src/scanner.rs", &[]),
        ]);
        let insight = infer(
            Path::new("/repo"),
            &[],
            &g,
            &["Axum".to_string()],
            &[],
            &Cache::off(),
        );
        assert_eq!(insight.confidence, Confidence::Low);
        assert!(insight.layer_chain().is_none());
        assert!(insight.evidence.iter().any(|e| e.contains("Axum")));
    }
    #[test]
    fn content_sign_names_the_line_that_matched() {
        let sign = |p: &str, t: &str| {
            content_layer(Path::new(p), t)
                .and_then(|v| v.into_iter().next())
                .map(|(_, s)| s)
        };
        let route = sign(
            "src/a.ts",
            "import x from 'y';\n\nrouter.get(\"/users\", list)",
        )
        .unwrap();
        assert_eq!(route.line, 3);
        assert!(route.sign.contains("/users"), "{}", route.sign);
        let model = sign("src/user.py", "import x\nclass User(models.Model):\n  pass").unwrap();
        assert_eq!(model.line, 2);
        let vue = sign("src/App.vue", "<template></template>").unwrap();
        assert_eq!((vue.line, vue.sign.as_str()), (0, ".vue component"));
    }

    #[test]
    fn classify_says_what_matched() {
        let matched = |p: &str| classify(Path::new(p)).map(|(_, m)| m);
        assert_eq!(matched("src/routes/users.ts").as_deref(), Some("routes/"));
        assert_eq!(
            matched("src/users/user.controller.ts").as_deref(),
            Some("user.controller.ts")
        );
    }

    #[test]
    fn placement_describes_file_and_sign() {
        let p = |placed_by: &str, matched: &str, code: Option<(usize, &str)>| Placement {
            file: "src/a.ts".to_string(),
            placed_by: placed_by.to_string(),
            matched: matched.to_string(),
            code: code.map(|(line, sign)| CodeSign {
                line,
                sign: sign.to_string(),
            }),
            name_said: None,
            also: Vec::new(),
        };
        assert_eq!(
            p("code", "", Some((14, "GET /users"))).describe(),
            "src/a.ts:14  GET /users"
        );
        assert_eq!(
            p("name", "routes/", Some((14, "GET /users"))).describe(),
            "src/a.ts:14  routes/; GET /users"
        );
        assert_eq!(p("name", "routes/", None).describe(), "src/a.ts  routes/");
        assert!(p("settings", "routes = \"src/\"", None)
            .describe()
            .contains(".codedna.toml"));
    }

    #[test]
    fn layers_and_flows_carry_examples() {
        let g = graph(&[
            ("src/routes/users.ts", &["src/services/users.ts"]),
            ("src/services/users.ts", &["src/models/user.ts"]),
            ("src/models/user.ts", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        let routes = &insight.layers[0];
        assert_eq!(routes.examples[0].file, "src/routes/users.ts");
        assert_eq!(routes.examples[0].placed_by, "name");
        assert!(insight
            .flows
            .iter()
            .any(|f| f.examples.iter().any(|e| e.from == "src/routes/users.ts")));
        assert!(insight
            .layers
            .iter()
            .all(|l| l.examples.len() <= MAX_EXAMPLES));
    }

    #[test]
    fn signs_in_strings_and_comments_are_not_evidence() {
        let at = |p: &str, t: &str| {
            content_layer(Path::new(p), t)
                .and_then(|v| v.into_iter().next())
                .map(|(i, _)| LAYERS[i].0)
        };
        let fixture =
            "export const samples = [\"@RestController\", 'pgTable(', `@Injectable()`];\n\
                       // app.get(\"/old\", h)\n/* (req, res, next) */\n\
                       const doc = \"router.post('/x', h)\";\n";
        assert_eq!(at("src/fixtures.ts", fixture), None);
        let py = "SQL = '''\nCREATE TABLE t (id int)\n'''\n# class U(models.Model)\n";
        assert_eq!(at("app/notes.py", py), None);
        let java = "class Doc { String s = \"@RestController\"; }";
        assert_eq!(at("src/Doc.java", java), None);
        // The real thing next to a string still counts.
        let mixed = "const label = \"pgTable(\";\nexport const t = pgTable(\"t\", {});\n";
        let (layer, sign) = content_layer(Path::new("src/t.ts"), mixed)
            .unwrap()
            .remove(0);
        assert_eq!((LAYERS[layer].0, sign.line), ("Data", 2));
    }

    #[test]
    fn content_shows_the_layer() {
        let at = |p: &str, t: &str| {
            content_layer(Path::new(p), t)
                .and_then(|v| v.into_iter().next())
                .map(|(i, _)| LAYERS[i].0)
        };
        assert_eq!(
            at("src/a.ts", "router.get(\"/users\", list)"),
            Some("Routes")
        );
        assert_eq!(
            at("app/x.py", "@app.get(\"/items\")\ndef items(): ..."),
            Some("Routes")
        );
        assert_eq!(
            at("src/auth.ts", "export function auth(req, res, next) {}"),
            Some("Middleware")
        );
        assert_eq!(
            at("src/user.py", "class User(models.Model):\n  pass"),
            Some("Data")
        );
        assert_eq!(
            at(
                "src/schema.ts",
                "export const users = pgTable(\"users\", {})"
            ),
            Some("Data")
        );
        assert_eq!(
            at("src/billing.ts", "@Injectable()\nexport class Billing {}"),
            Some("Services")
        );
        assert_eq!(
            at("src/Card.tsx", "export const Card = () => <div />"),
            Some("UI")
        );
        assert_eq!(at("src/Page.svelte", ""), Some("UI"));
        assert_eq!(
            at(
                "Api/Orders.cs",
                "[ApiController]\npublic class Orders : ControllerBase {}"
            ),
            Some("Routes")
        );
        assert_eq!(
            at(
                "Data/Shop.cs",
                "class Shop : DbContext { DbSet<Order> Orders; }"
            ),
            Some("Data")
        );
        assert_eq!(at("src/hooks.tsx", "export const useX = () => 1"), None);
        assert_eq!(
            at("src/strings.ts", "export const trim = (s) => s.trim()"),
            None
        );
    }

    #[test]
    fn file_between_routes_and_data_is_a_service() {
        let g = graph(&[
            ("src/routes/users.ts", &["src/lib/users.ts"]),
            (
                "src/routes/posts.ts",
                &["src/lib/posts.ts", "src/lib/serializers.ts"],
            ),
            ("src/lib/serializers.ts", &["src/models/post.ts"]),
            ("src/lib/users.ts", &["src/models/user.ts"]),
            ("src/lib/posts.ts", &["src/models/post.ts"]),
            ("src/lib/strings.ts", &[]),
            ("src/models/user.ts", &[]),
            ("src/models/post.ts", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[], &Cache::off());
        let services = insight
            .layers
            .iter()
            .find(|l| l.name == "Services")
            .unwrap();
        assert_eq!(services.files, 2);
        assert_eq!(services.placed_by_imports, 2);
        assert_eq!(
            insight.layer_chain().as_deref(),
            Some("Routes → Services → Data")
        );
        assert!(insight
            .evidence
            .iter()
            .any(|e| e.contains("placed in Services")));
    }

    #[test]
    fn code_and_settings_classify_unusual_layouts() {
        let root = std::env::temp_dir().join(format!(
            "codedna_arch_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("pkg/a.ts", "app.get(\"/a\", h)");
        write("pkg/b.ts", "app.post(\"/b\", h)");
        write("pkg/c.ts", "export const t = pgTable(\"t\", {})");
        write("pkg/d.ts", "export const u = pgTable(\"u\", {})");
        write("odd/e.ts", "export const e = 1");
        write("odd/f.ts", "export const f = 1");
        write(SETTINGS_FILE, "[layers]\nmiddleware = [\"odd/\"]\n");
        let p = |r: &str| root.join(r);
        let g: HashMap<PathBuf, Vec<PathBuf>> = [
            (p("pkg/a.ts"), vec![p("pkg/c.ts")]),
            (p("pkg/b.ts"), vec![]),
            (p("pkg/c.ts"), vec![]),
            (p("pkg/d.ts"), vec![]),
            (p("odd/e.ts"), vec![]),
            (p("odd/f.ts"), vec![]),
        ]
        .into_iter()
        .collect();
        let insight = infer(&root, &[], &g, &[], &[], &Cache::off());
        std::fs::remove_dir_all(&root).ok();
        let names: Vec<&str> = insight.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Routes", "Middleware", "Data"]);
        assert_eq!(insight.layers[0].confirmed_by_code, 2);
        assert_eq!(insight.layers[1].confirmed_by_code, 0);
        assert!(insight.evidence[0].contains(SETTINGS_FILE));
        // Middleware came from settings, not code, so this is not code-backed
        // and one import is not enough on its own.
        assert_eq!(insight.confidence, Confidence::Medium);
    }

    #[test]
    fn code_overrules_folder_names() {
        let root = std::env::temp_dir().join(format!(
            "codedna_arch_code_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        // Tables kept in `services/` are data; a handler in `routes/` with
        // the middleware signature stays a route.
        write(
            "src/services/user.ts",
            "export const u = pgTable(\"u\", {})",
        );
        write(
            "src/services/order.ts",
            "export const o = pgTable(\"o\", {})",
        );
        write(
            "src/routes/users.ts",
            "export function list(req, res, next) {}\nconst k = NextFunction;",
        );
        write("src/routes/orders.ts", "app.get(\"/orders\", h)");
        let p = |r: &str| root.join(r);
        let g: HashMap<PathBuf, Vec<PathBuf>> = [
            (p("src/services/user.ts"), vec![]),
            (p("src/services/order.ts"), vec![]),
            (p("src/routes/users.ts"), vec![p("src/services/user.ts")]),
            (p("src/routes/orders.ts"), vec![p("src/services/order.ts")]),
        ]
        .into_iter()
        .collect();
        let insight = infer(&root, &[], &g, &[], &[], &Cache::off());
        std::fs::remove_dir_all(&root).ok();
        let names: Vec<&str> = insight.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Routes", "Data"]);
        let data = &insight.layers[1];
        assert_eq!(data.confirmed_by_code, 2);
        let example = &data.examples[0];
        assert_eq!(example.placed_by, "code");
        assert_eq!(example.name_said.as_deref(), Some("services/ (Services)"));
        assert!(
            example
                .describe()
                .ends_with("pgTable(; not services/ (Services)"),
            "{}",
            example.describe()
        );
        assert_eq!(insight.layers[0].files, 2);
    }
}
