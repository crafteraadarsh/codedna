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
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
const MAX_EVIDENCE: usize = 8;

/// Most example files kept per layer, and imports per flow.
const MAX_EXAMPLES: usize = 3;

/// Infer layers from file locations, file contents and the import graph.
///
/// Paths in `graph` may be absolute; `root` is stripped for display.
pub fn infer(
    root: &Path,
    files: &[PathBuf],
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    frameworks: &[String],
    databases: &[String],
) -> ArchitectureInsight {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
    let overrides = Overrides::load(root);

    // Every source file, including languages whose imports are not followed.
    let mut nodes: Vec<&PathBuf> = graph.keys().collect();
    nodes.extend(files.iter().filter(|f| {
        !graph.contains_key(*f)
            && f.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                let e = e.to_ascii_lowercase();
                e == "astro" || crate::runbook::CODE_EXTENSIONS.contains(&e.as_str())
            })
    }));
    let no_deps: Vec<PathBuf> = Vec::new();
    let deps_of = |file: &PathBuf| graph.get(file).unwrap_or(&no_deps);

    // What each file's code does, read once.
    let by_code: HashMap<&PathBuf, (usize, CodeSign)> = nodes
        .par_iter()
        .filter_map(|&file| {
            let relative = rel(file);
            if is_test(&relative) {
                return None;
            }
            let text = read_capped(file)?;
            content_layer(&relative, &text).map(|found| (file, found))
        })
        .collect();

    // Settings first, then code, then names: a folder or file name is a
    // convention, the code is what the file does.
    // Each placed file: (layer, what placed it, what matched).
    let mut layer_of: HashMap<&PathBuf, (usize, Source, String)> = HashMap::new();
    // Files whose code overruled their name, and what the name suggested.
    let mut name_said: HashMap<&PathBuf, String> = HashMap::new();
    for &file in &nodes {
        let relative = rel(file);
        if let Some((index, prefix)) = overrides.layer(&relative) {
            layer_of.insert(file, (index, Source::Settings, prefix));
            continue;
        }
        let code = by_code.get(file);
        let found = match (classify(&relative), code) {
            (Some((name_index, name)), Some((code_index, sign)))
                if code_wins(*code_index, name_index) =>
            {
                if *code_index != name_index {
                    name_said.insert(file, format!("{name} ({})", LAYERS[name_index].0));
                }
                (*code_index, Source::Code, sign.sign.clone())
            }
            (Some((index, name)), _) => (index, Source::Name, name),
            (None, Some((index, sign))) => (*index, Source::Code, sign.sign.clone()),
            (None, None) => continue,
        };
        layer_of.insert(file, found);
    }

    // A file nothing else placed, imported by routes or middleware and
    // importing data, is the service layer between them.
    let mut importers: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();
    for (file, deps) in graph {
        for dep in deps {
            importers.entry(dep).or_default().push(file);
        }
    }
    let in_layer = |file: &PathBuf, layers: &[usize]| {
        layer_of
            .get(file)
            .is_some_and(|(index, _, _)| layers.contains(index))
    };
    let between: Vec<(&PathBuf, String)> = nodes
        .iter()
        .copied()
        .filter(|file| {
            let relative = rel(file);
            !layer_of.contains_key(file) && !is_test(&relative) && !is_shape(&relative)
        })
        .filter_map(|file| {
            let by = importers
                .get(file)?
                .iter()
                .filter(|f| in_layer(f, &[1, 2]))
                .min()?;
            let data = deps_of(file).iter().filter(|d| in_layer(d, &[4])).min()?;
            Some((
                file,
                format!(
                    "imported by {}, imports {}",
                    display_path(root, by),
                    display_path(root, data)
                ),
            ))
        })
        .collect();
    for (file, matched) in between {
        layer_of.insert(file, (3, Source::Imports, matched));
    }

    let mut dir_counts: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); LAYERS.len()];
    let mut confirmed = vec![0usize; LAYERS.len()];
    let mut placed = vec![0usize; LAYERS.len()];
    let mut from_settings = 0usize;
    let mut examples: Vec<Vec<Placement>> = vec![Vec::new(); LAYERS.len()];
    for (file, (index, source, matched)) in &layer_of {
        let (index, source) = (*index, *source);
        let relative = rel(file);
        let dir = relative
            .parent()
            .map(|d| sanitize(&d.to_string_lossy().replace('\\', "/")))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        *dir_counts[index].entry(dir).or_default() += 1;
        let code = by_code
            .get(file)
            .filter(|(code_index, _)| *code_index == index)
            .map(|(_, sign)| sign.clone());
        if code.is_some() {
            confirmed[index] += 1;
        }
        examples[index].push(Placement {
            file: display_path(root, file),
            placed_by: source.as_str().to_string(),
            matched: matched.clone(),
            code,
            name_said: name_said.get(file).cloned(),
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
    let code_backed =
        layers.len() >= 2 && layers.iter().all(|l| l.confirmed_by_code > 0) && upward <= downward;
    let confidence = if (layers.len() >= 2 && downward >= 2 && downward > upward) || code_backed {
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
    if layers.is_empty() {
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
    evidence.truncate(MAX_EVIDENCE);

    ArchitectureInsight {
        layers,
        flows,
        confidence,
        evidence,
    }
}

/// Whether a file's code layer overrules the layer its name suggests.
///
/// Code wins, except that request handlers in a routes folder share the
/// middleware signature (`(req, res, next)`, `NextFunction`) without being
/// middleware.
fn code_wins(code_index: usize, name_index: usize) -> bool {
    !(code_index == 2 && name_index == 1)
}

/// Layer index for a relative path, from its file name first, then its
/// closest matching folder. Tests mirror the layout they test, so they are
/// left out.
fn classify(path: &Path) -> Option<(usize, String)> {
    if is_test(path) {
        return None;
    }
    let is_ui_file = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| UI_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    match classify_by_name(path)? {
        // File-based routes written as components (`routes/index.tsx`) are UI.
        (1, name) if is_ui_file => Some((0, name)),
        found => Some(found),
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

/// Layer index and the file name (`user.controller.ts`) or folder
/// (`routes/`) that marks it.
fn classify_by_name(path: &Path) -> Option<(usize, String)> {
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
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            return Some((index, sanitize(&file)));
        }
    }

    path.parent()?
        .iter()
        .rev()
        .filter_map(|c| c.to_str())
        .find_map(|dir| {
            layer_for_name(&dir.to_ascii_lowercase(), is_ui_file)
                .map(|index| (index, format!("{}/", sanitize(dir))))
        })
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
// Evidence from code and settings
// ---------------------------------------------------------------------------

/// What placed a file in its layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Settings,
    Name,
    Code,
    Imports,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Settings => "settings",
            Source::Name => "name",
            Source::Code => "code",
            Source::Imports => "imports",
        }
    }
}

/// What the code of each layer shows, for evidence lines.
const LAYER_SIGNS: &[&str] = &[
    "renders components",
    "defines routes",
    "wraps requests",
    "declares services",
    "defines models or tables",
];

/// Per-repository settings file.
pub const SETTINGS_FILE: &str = ".codedna.toml";

/// Largest settings file read.
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// Largest source file read for content evidence.
const MAX_CODE_BYTES: u64 = 512 * 1024;

/// Middleware: functions that sit around a request.
const MIDDLEWARE_SIGNS: &[&str] = &[
    "(req, res, next)",
    "NextFunction",
    "implements NestMiddleware",
    "implements CanActivate",
    "implements HttpInterceptor",
    "MiddlewareMixin",
    "BaseHTTPMiddleware",
    "func(next http.Handler) http.Handler",
    "middleware::from_fn",
    "HandlerInterceptor",
    "OncePerRequestFilter",
];

/// Data: model, table and repository definitions (not every query, so a
/// service that calls the database stays a service).
const DATA_SIGNS: &[&str] = &[
    "@Entity(",
    "(models.Model)",
    "declarative_base(",
    "(DeclarativeBase)",
    "mapped_column(",
    "(SQLModel, table=True)",
    "new Schema(",
    "mongoose.Schema(",
    "mongoose.model(",
    "sequelize.define(",
    "#[derive(Queryable",
    "derive(sqlx::FromRow",
    "diesel::table!",
    "gorm.Model",
    "@Table(",
    "extends Model",
    "< ApplicationRecord",
    "< ActiveRecord::Base",
    "CREATE TABLE",
    "@Repository",
    "extends JpaRepository",
    "pgTable(",
    "sqliteTable(",
    "mysqlTable(",
    ": DbContext",
    "DbSet<",
];

/// Services: classes a framework injects.
const SERVICE_SIGNS: &[&str] = &["@Service", "@Injectable(", "@Transactional"];

/// Controllers whose routes come from attributes or annotations on the class.
const CONTROLLER_SIGNS: &[&str] = &[
    "[ApiController]",
    ": ControllerBase",
    "[HttpGet",
    "[HttpPost",
    "@RestController",
    "@Controller",
];

/// Files that describe data shapes rather than work: never placed in
/// Services by their imports alone.
const SHAPE_NAMES: &[&str] = &[
    "serializer",
    "schema",
    "dto",
    "types",
    "type",
    "util",
    "helper",
    "const",
    "config",
    "settings",
    "renderer",
];

fn is_shape(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    SHAPE_NAMES.iter().any(|n| stem.contains(n))
}

/// Read a source file for content evidence, skipping very large ones.
fn read_capped(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_CODE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Layer index shown by a file's code, if any, with what matched and where.
fn content_layer(path: &Path, text: &str) -> Option<(usize, CodeSign)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "vue" | "svelte" | "astro") {
        let sign = CodeSign {
            line: 0,
            sign: format!(".{ext} component"),
        };
        return Some((0, sign));
    }
    // Signs count only in code, not in strings, comments or fixtures.
    let original = text;
    let code = crate::code_text::code_only(&ext, text);
    let text = code.as_str();
    if matches!(ext.as_str(), "tsx" | "jsx") {
        let markup = [text.find("/>"), text.find("</")]
            .into_iter()
            .flatten()
            .min();
        if let Some(at) = markup {
            return Some((0, sign_at(text, at, "JSX markup")));
        }
    }
    let first = |signs: &[&'static str]| {
        signs
            .iter()
            .filter_map(|s| text.find(s).map(|at| (at, *s)))
            .min()
            .map(|(at, s)| sign_at(text, at, s))
    };
    let file = path.to_string_lossy().replace('\\', "/");
    if let Some(route) = crate::runbook::routes(&file, original).into_iter().next() {
        let sign = CodeSign {
            line: route.line,
            sign: sanitize(&format!("{} {}", route.method, route.path)),
        };
        return Some((1, sign));
    }
    if let Some(sign) = first(CONTROLLER_SIGNS) {
        return Some((1, sign));
    }
    if let Some(sign) = first(MIDDLEWARE_SIGNS) {
        return Some((2, sign));
    }
    if let Some(sign) = first(DATA_SIGNS) {
        return Some((4, sign));
    }
    first(SERVICE_SIGNS).map(|sign| (3, sign))
}

/// A sign found at byte `at` of `text`.
fn sign_at(text: &str, at: usize, sign: &str) -> CodeSign {
    CodeSign {
        line: text[..at].bytes().filter(|b| *b == b'\n').count() + 1,
        sign: sign.to_string(),
    }
}

/// Layer folders set by hand in `.codedna.toml`:
///
/// ```toml
/// [layers]
/// routes = ["src/http/"]
/// data = ["src/store/", "src/tables.rs"]
/// ```
///
/// Keys are `ui`, `routes`, `middleware`, `services` and `data`; values are
/// path prefixes relative to the repository root. The longest match wins.
#[derive(Debug, Default)]
struct Overrides {
    prefixes: Vec<(String, usize)>,
}

impl Overrides {
    fn load(root: &Path) -> Self {
        let path = root.join(SETTINGS_FILE);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return Self::default();
        };
        if !meta.is_file() || meta.len() > MAX_SETTINGS_BYTES {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let Ok(table) = text.parse::<toml::Table>() else {
            return Self::default();
        };
        let Some(layers) = table.get("layers").and_then(|v| v.as_table()) else {
            return Self::default();
        };
        let mut prefixes = Vec::new();
        for (key, value) in layers {
            let Some(index) = LAYERS
                .iter()
                .position(|(name, _)| name.eq_ignore_ascii_case(key))
            else {
                continue;
            };
            let Some(list) = value.as_array() else {
                continue;
            };
            for prefix in list.iter().filter_map(|v| v.as_str()) {
                let prefix = normalise(prefix);
                if !prefix.is_empty() {
                    prefixes.push((prefix, index));
                }
            }
        }
        prefixes.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Self { prefixes }
    }

    /// Layer index and the `[layers]` entry that matched.
    fn layer(&self, relative: &Path) -> Option<(usize, String)> {
        if self.prefixes.is_empty() {
            return None;
        }
        let path = normalise(&relative.to_string_lossy());
        self.prefixes
            .iter()
            .find(|(prefix, _)| {
                path == *prefix.trim_end_matches('/')
                    || path.starts_with(prefix.as_str())
                        && (prefix.ends_with('/') || path[prefix.len()..].starts_with('/'))
            })
            .map(|(prefix, index)| {
                let key = LAYERS[*index].0.to_ascii_lowercase();
                (*index, sanitize(&format!("{key} = \"{prefix}\"")))
            })
    }
}

/// `./src\\x` → `src/x`.
fn normalise(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    path.trim_start_matches('/').to_string()
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
        assert_eq!(insight.confidence, Confidence::Medium);
        assert!(insight.evidence.iter().any(|e| e.contains("against")));
    }

    #[test]
    fn lone_file_without_imports_is_not_a_layer() {
        let g = graph(&[("src/git_handler.rs", &[]), ("src/main.rs", &[])]);
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
        assert!(insight.layers.is_empty());
        assert_eq!(insight.confidence, Confidence::Low);
    }

    #[test]
    fn no_layers_falls_back_to_technologies() {
        let g = graph(&[
            ("src/main.rs", &["src/scanner.rs"]),
            ("src/scanner.rs", &[]),
        ]);
        let insight = infer(Path::new("/repo"), &[], &g, &["Axum".to_string()], &[]);
        assert_eq!(insight.confidence, Confidence::Low);
        assert!(insight.layer_chain().is_none());
        assert!(insight.evidence.iter().any(|e| e.contains("Axum")));
    }
    #[test]
    fn content_sign_names_the_line_that_matched() {
        let sign = |p: &str, t: &str| content_layer(Path::new(p), t).map(|(_, s)| s);
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
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
        let at = |p: &str, t: &str| content_layer(Path::new(p), t).map(|(i, _)| LAYERS[i].0);
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
        let (layer, sign) = content_layer(Path::new("src/t.ts"), mixed).unwrap();
        assert_eq!((LAYERS[layer].0, sign.line), ("Data", 2));
    }

    #[test]
    fn content_shows_the_layer() {
        let at = |p: &str, t: &str| content_layer(Path::new(p), t).map(|(i, _)| LAYERS[i].0);
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
    fn settings_place_files_by_prefix() {
        let o = Overrides::parse(
            "[layers]\nroutes = [\"./src/http/\"]\nData = [\"src/store\", \"src/http/db/\"]\nnonsense = [\"x/\"]\n",
        );
        let at = |p: &str| o.layer(Path::new(p)).map(|(i, _)| LAYERS[i].0);
        assert_eq!(at("src/http/users.rs"), Some("Routes"));
        assert_eq!(at("src/http/db/pool.rs"), Some("Data"));
        assert_eq!(at("src/store/mod.rs"), Some("Data"));
        assert_eq!(at("src/store"), Some("Data"));
        assert_eq!(at("src/storefront/a.rs"), None);
        assert_eq!(at("x/a.rs"), None);
        assert!(Overrides::parse("not toml [[").prefixes.is_empty());
        assert!(Overrides::parse("[other]\na = 1").prefixes.is_empty());
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
        let insight = infer(Path::new("/repo"), &[], &g, &[], &[]);
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
        let insight = infer(&root, &[], &g, &[], &[]);
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
        let insight = infer(&root, &[], &g, &[], &[]);
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
