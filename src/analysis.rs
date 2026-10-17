//! Analysis aggregation module.
//!
//! Wires every analysis module together and produces a single `AnalysisResult`
//! that represents the complete intelligence report for a repository.

use crate::architecture::{self, ArchitectureInsight};
use crate::cache::Cache;
use crate::coverage::{self, Coverage};
use crate::cycles::{self, Cycle};
use crate::dead_code_detector::{self, DeadCodeCandidate, UsageReport};
use crate::dependency_graph;
use crate::facts::{self, DependencyFact, DependencyFacts};
use crate::framework_detector;
use crate::frameworks;
use crate::graph;
use crate::language_detector;
use crate::loc_counter;
use crate::scanner;
use crate::workspaces::{self, Workspace};
use rayon::prelude::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Per-file metadata produced by the analysis pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileInfo {
    /// Path to the file (relative to the scanned root where possible).
    pub file: PathBuf,
    /// Code lines in the file: comments and blank lines are not counted.
    pub loc: usize,
    /// Detected language name (e.g. "TypeScript", "Python").
    pub language: String,
}

/// Version of the JSON report's shape. It goes up only when a field is
/// removed, renamed or changes meaning; new fields keep the same version.
pub const SCHEMA_VERSION: u32 = 1;

/// The tool that produced a report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Analyzer {
    pub name: String,
    pub version: String,
}

/// The JSON Schema of the report, as written to `schema/report.schema.json`.
pub fn json_schema() -> String {
    let schema = schemars::schema_for!(AnalysisResult);
    serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_string())
}

impl Analyzer {
    /// This build of CodeDna.
    pub fn current() -> Self {
        Analyzer {
            name: "codedna".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Aggregated intelligence report for a repository.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AnalysisResult {
    /// Version of the report's shape; see [`SCHEMA_VERSION`]. Reports from
    /// before versioning read as 0.
    #[serde(default)]
    pub schema_version: u32,

    /// Name and version of the tool that wrote the report. Empty in
    /// reports from before versioning.
    #[serde(default)]
    pub analyzer: Analyzer,

    /// High-level project classification (e.g. "Full-stack web application").
    pub project_type: String,

    /// Total code lines across all recognised source files.  Comment-only
    /// and blank lines are excluded; see `comment_lines` and `blank_lines`.
    pub total_loc: usize,

    /// Comment-only lines across all recognised source files.
    #[serde(default)]
    pub comment_lines: usize,

    /// Blank lines across all recognised source files.
    #[serde(default)]
    pub blank_lines: usize,

    /// LOC per language: `{ "TypeScript": 12450, "Python": 800, … }`.
    #[serde(serialize_with = "serialize_sorted")]
    pub languages: HashMap<String, usize>,

    /// Detected application frameworks (e.g. `["React", "Express"]`).
    pub frameworks: Vec<String>,

    /// Detected databases / data-layer technologies (e.g. `["PostgreSQL", "Redis"]`).
    pub databases: Vec<String>,

    /// Detected infrastructure / DevOps tooling (e.g. `["Docker", "GitHub Actions"]`).
    pub infrastructure: Vec<String>,

    /// Inferred architecture description
    /// (e.g. `"Frontend → API → Database"`).
    pub architecture: String,

    /// Layers found in the code, the imports between them, a confidence
    /// level and the evidence behind it.
    #[serde(default)]
    pub architecture_details: ArchitectureInsight,

    /// Files unreachable from any entry point via dependency graph traversal.
    pub dead_code: Vec<PathBuf>,

    /// The same files with a confidence level and the reason for it, most
    /// certain first.
    #[serde(default)]
    pub dead_code_details: Vec<DeadCodeCandidate>,

    /// How each file is used: confirmed from the program's entry points,
    /// possibly, for types only, by tests only or by tooling only, with the
    /// path from the entry that reaches it; unreachable files are the
    /// dead-code candidates.
    #[serde(default)]
    pub usage: UsageReport,

    /// Groups of files that import each other, each as its shortest cycle
    /// (the last file imports the first), shortest first.
    #[serde(default)]
    pub circular_imports: Vec<Vec<PathBuf>>,

    /// The same cycles, in the same order, each with whether it closes
    /// through runtime imports only, needs a possible edge, or needs a
    /// type-only import.
    #[serde(default)]
    pub cycle_details: Vec<Cycle>,

    /// Workspaces (monorepo packages) declared at the root, if any.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,

    /// Directed import graph: `file → [files it imports]`. A projection of
    /// `dependencies`: the targets of its resolved facts, per source file.
    #[serde(serialize_with = "serialize_dep_graph")]
    #[serde(deserialize_with = "deserialize_dep_graph")]
    pub dependency_graph: HashMap<PathBuf, Vec<PathBuf>>,

    /// Typed dependency facts: every import, re-export, worker, process,
    /// configured entry and possible dynamic load found, with its kind,
    /// resolution, certainty and evidence. Unresolved and external imports
    /// are kept too. Sorted by source file and position.
    #[serde(default)]
    pub dependencies: DependencyFacts,

    /// How much of the code the dependency analysis covered: files whose
    /// imports were followed, imports that did not resolve and languages
    /// that were not analyzed. Dead code and cycles only speak for the
    /// covered part.
    #[serde(default)]
    pub coverage: Coverage,

    /// Per-file breakdown, sorted by LOC descending.
    pub file_breakdown: Vec<FileInfo>,
}

impl AnalysisResult {
    /// Dead-code candidates with confidence, most certain first.
    pub fn dead_code_candidates(&self) -> Vec<DeadCodeCandidate> {
        dead_code_detector::candidates_of(&self.dead_code, &self.dead_code_details)
    }

    /// Circular imports with their kind; `None` for results saved before
    /// kinds were recorded.
    pub fn cycle_kinds(&self) -> Vec<Option<cycles::CycleKind>> {
        if self.cycle_details.len() == self.circular_imports.len() {
            self.cycle_details.iter().map(|c| Some(c.kind)).collect()
        } else {
            vec![None; self.circular_imports.len()]
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Lockfiles written by package managers. They are generated, so counting
/// them would swamp the real code in line totals and language shares.
const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "Cargo.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "composer.lock",
    "Gemfile.lock",
    "go.sum",
    "flake.lock",
];

/// Whether `file` is a package-manager lockfile.
pub(crate) fn is_lockfile(file: &Path) -> bool {
    file.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| LOCKFILES.contains(&n))
}

/// Run the full analysis pipeline on `root` and return an `AnalysisResult`.
///
/// # Pipeline steps
/// 1. Scan repository files (respecting ignore rules)
/// 2. Count LOC per file; skip lockfiles and binary / unreadable files
/// 3. Detect language per file; build aggregated language LOC map
/// 4. Detect frameworks and databases from manifest files
/// 5. Build dependency graph from import/require statements
/// 6. Detect dead code via BFS from entry points
/// 7. Infer project type and architecture from collected signals
/// 8. List workspace packages declared at the root, each with its own
///    size, languages, stack, dead files and links to other packages
pub fn analyze(root: &Path) -> AnalysisResult {
    // ── 1. Scan ──────────────────────────────────────────────────────────────
    let files = scanner::scan_repository(root);
    let cache = Cache::open(root, &files);

    // ── 2 & 3. LOC + language (parallel) ────────────────────────────────────
    // Each file is processed independently — ideal for rayon par_iter.
    let per_file: Vec<(PathBuf, loc_counter::LineCounts, String)> = files
        .par_iter()
        .filter(|file| !is_lockfile(file))
        .filter_map(|file| {
            let counts = cache.loc(file, || loc_counter::count_line_kinds(file))?;
            let language = language_detector::detect_language(file)
                .map(|l| l.to_string())
                .unwrap_or_else(|| "Other".to_string());
            Some((file.clone(), counts, language))
        })
        .collect();

    // Comment and blank totals cover the same files as `total_loc`: those in
    // a recognised language.
    let (comment_lines, blank_lines) = per_file
        .iter()
        .filter(|(file, _, _)| language_detector::detect_language(file).is_some())
        .fold((0, 0), |(c, b), (_, k, _)| (c + k.comment, b + k.blank));

    let mut file_loc_pairs: Vec<(PathBuf, usize)> = Vec::with_capacity(per_file.len());
    let mut file_breakdown: Vec<FileInfo> = Vec::with_capacity(per_file.len());

    for (file, counts, language) in per_file {
        let loc = counts.code;
        file_loc_pairs.push((file.clone(), loc));
        file_breakdown.push(FileInfo {
            file,
            loc,
            language,
        });
    }

    // Sort file breakdown by LOC descending for at-a-glance readability.
    file_breakdown.sort_unstable_by_key(|a| std::cmp::Reverse(a.loc));

    // Build language → total LOC map.
    let lang_map = language_detector::build_language_map(&file_loc_pairs);
    let languages: HashMap<String, usize> = lang_map
        .into_iter()
        .map(|(lang, loc)| (lang.to_string(), loc))
        .collect();

    let total_loc: usize = languages.values().sum();

    // ── 4. Frameworks + databases + infrastructure ────────────────────────────
    let fw_result = framework_detector::detect_frameworks(&files);
    let infrastructure = framework_detector::detect_infrastructure(&files);

    // ── 5. Dependency graph ───────────────────────────────────────────────────
    let (dep_graph, import_tallies, dependency_facts) =
        dependency_graph::build_facts(&files, &cache);
    let coverage = coverage::measure(root, &files, &import_tallies);

    // ── 6. Dead code ──────────────────────────────────────────────────────────
    let adapters = frameworks::detect(&files);
    let typed_graph = graph::typed_edges(dep_graph.keys().cloned(), &dependency_facts);
    let usage = dead_code_detector::classify(
        &typed_graph,
        &dead_code_detector::entry_roles(&adapters.entries),
        &adapters.next_roots,
    );
    let dead_code = usage.unreachable();

    // ── 7. Inference ──────────────────────────────────────────────────────────
    let project_type = infer_project_type(
        &fw_result.frameworks,
        &fw_result.databases,
        &languages,
        &infrastructure,
    );
    let architecture = infer_architecture(&fw_result.frameworks, &fw_result.databases);
    let architecture_details = architecture::infer(
        root,
        &files,
        &dep_graph,
        &fw_result.frameworks,
        &fw_result.databases,
        &cache,
    );

    // ── 8. Normalise paths ──────────────────────────────────────────────────
    // Strip the `root` prefix so output is always relative, regardless of
    // whether the root was a local `.` or an absolute temp-dir path.
    let make_relative = |p: &Path| relative_path(root, p);

    let uncertain =
        dead_code_detector::uncertainty(&dependency_facts, &usage, &typed_graph, make_relative);
    let usage = usage.report(make_relative);
    let dead_code_details =
        dead_code_detector::assess_with(root, &dead_code, &files, &dep_graph, &uncertain, &cache)
            .into_iter()
            .map(|c| DeadCodeCandidate {
                file: make_relative(&c.file),
                ..c
            })
            .collect();
    cache.save();
    let dead_code = dead_code
        .into_iter()
        .map(|p| make_relative(&p))
        .collect::<Vec<PathBuf>>();

    let dependency_graph: HashMap<PathBuf, Vec<PathBuf>> = dep_graph
        .into_iter()
        .map(|(k, vs)| {
            let key = make_relative(&k);
            let vals = vs.iter().map(|v| make_relative(v)).collect();
            (key, vals)
        })
        .collect();
    let mut facts: Vec<DependencyFact> = dependency_facts
        .into_iter()
        .map(|f| f.map_paths(make_relative))
        .collect();
    facts::sort(&mut facts);
    let mut entries: Vec<DependencyFact> = adapters
        .entries
        .into_iter()
        .map(|f| f.map_paths(make_relative))
        .collect();
    facts::sort(&mut entries);
    let dependencies = DependencyFacts {
        rules_version: facts::RULES_VERSION,
        facts,
        entries,
        adapters: adapters
            .active
            .into_iter()
            .map(|a| a.map_paths(make_relative))
            .collect(),
    };
    let cycle_details = cycles::find_typed_cycles(&graph::typed_edges(
        dependency_graph.keys().cloned(),
        &dependencies.facts,
    ));
    let circular_imports = cycle_details.iter().map(|c| c.files.clone()).collect();

    let file_breakdown = file_breakdown
        .into_iter()
        .map(|fi| FileInfo {
            file: make_relative(&fi.file),
            loc: fi.loc,
            language: fi.language,
        })
        .collect::<Vec<FileInfo>>();

    // ── 9. Workspace packages ───────────────────────────────────────────────
    let mut workspaces = workspaces::detect(root);
    if !workspaces.is_empty() {
        let file_loc: Vec<(PathBuf, usize, String)> = file_breakdown
            .iter()
            .map(|f| (f.file.clone(), f.loc, f.language.clone()))
            .collect();
        workspaces::profile(
            root,
            &mut workspaces,
            &workspaces::RepoFacts {
                files: &files,
                file_loc: &file_loc,
                dead_code: &dead_code,
                dependency_graph: &dependency_graph,
            },
        );
    }

    AnalysisResult {
        schema_version: SCHEMA_VERSION,
        analyzer: Analyzer::current(),
        project_type,
        total_loc,
        comment_lines,
        blank_lines,
        languages,
        frameworks: fw_result.frameworks,
        databases: fw_result.databases,
        infrastructure,
        architecture,
        architecture_details,
        dead_code,
        dead_code_details,
        usage,
        circular_imports,
        cycle_details,
        workspaces,
        dependency_graph,
        dependencies,
        coverage,
        file_breakdown,
    }
}

// ---------------------------------------------------------------------------
// Inference helpers
// ---------------------------------------------------------------------------

/// Infer a human-readable project type from the detected signals.
fn infer_project_type(
    frameworks: &[String],
    databases: &[String],
    languages: &HashMap<String, usize>,
    infrastructure: &[String],
) -> String {
    let has_frontend = has_any(
        frameworks,
        &[
            "React", "Vue", "Next.js", "Nuxt", "Svelte", "Astro", "Remix", "Gatsby", "Angular",
        ],
    );
    let has_backend = has_any(
        frameworks,
        &[
            "Express",
            "FastAPI",
            "Django",
            "Flask",
            "Axum",
            "Actix-web",
            "Rocket",
            "NestJS",
            "Fastify",
            "Koa",
            "Gin",
            "Echo",
            "Fiber",
            "Tokio",
        ],
    );
    let has_db = !databases.is_empty();

    let has_rust = languages.contains_key("Rust");
    let has_solidity = languages.contains_key("Solidity");
    let has_go = languages.contains_key("Go");
    let has_python = languages.contains_key("Python");

    if has_solidity {
        return "Blockchain / Smart-contract project".to_string();
    }

    if has_frontend && has_backend && has_db {
        return "Full-stack web application".to_string();
    }

    if has_frontend && has_backend {
        return "Full-stack web application (no detected database)".to_string();
    }

    if has_frontend {
        return "Frontend web application".to_string();
    }

    if has_backend && has_db {
        if has_rust {
            return "Rust backend service".to_string();
        }
        if has_go {
            return "Go backend service".to_string();
        }
        if has_python {
            return "Python backend service".to_string();
        }
        return "Backend API service".to_string();
    }

    if has_backend {
        if has_rust {
            return "Rust backend service".to_string();
        }
        return "Backend API service".to_string();
    }

    if has_rust {
        return "Rust library / CLI tool".to_string();
    }

    if has_go {
        return "Go application".to_string();
    }

    if has_python {
        return "Python application".to_string();
    }

    // Containerised service (Docker present but no clear framework detected)
    if infrastructure.contains(&"Docker".to_string()) {
        return "Containerised service".to_string();
    }

    "Unknown / General-purpose project".to_string()
}

/// Infer an architecture description from the detected frameworks and databases.
fn infer_architecture(frameworks: &[String], databases: &[String]) -> String {
    let has_frontend = has_any(
        frameworks,
        &[
            "React", "Vue", "Next.js", "Nuxt", "Svelte", "Astro", "Remix", "Gatsby", "Angular",
        ],
    );
    let has_backend = has_any(
        frameworks,
        &[
            "Express",
            "FastAPI",
            "Django",
            "Flask",
            "Axum",
            "Actix-web",
            "Rocket",
            "NestJS",
            "Fastify",
            "Koa",
            "Gin",
            "Echo",
            "Fiber",
            "Tokio",
        ],
    );
    let has_db = !databases.is_empty();

    match (has_frontend, has_backend, has_db) {
        (true, true, true) => "Frontend → API → Database".to_string(),
        (true, true, false) => "Frontend → API".to_string(),
        (false, true, true) => "API → Database".to_string(),
        (true, false, true) => "Frontend → Database".to_string(),
        (true, false, false) => "Frontend only".to_string(),
        (false, true, false) => "API only".to_string(),
        _ => "Monolithic / undetermined".to_string(),
    }
}

/// Return `true` if any item in `haystack` is found in `needles`.
fn has_any(haystack: &[String], needles: &[&str]) -> bool {
    haystack.iter().any(|item| needles.contains(&item.as_str()))
}

// ---------------------------------------------------------------------------
// Serde helpers for HashMap<PathBuf, Vec<PathBuf>>
// ---------------------------------------------------------------------------
//
// JSON object keys must be strings, so we serialise PathBuf as its UTF-8
// display form and deserialise back.

/// `path` relative to `root`, written `./dir/file`. `.` components are
/// ignored on both sides, so `tokio/src/lib.rs` and `./tokio/src/lib.rs`
/// under the root `.` give the same answer. A path outside the root is
/// returned unchanged.
/// `path` (already relative, `/`-separated) written as `./path`, the form
/// every path in CodeDna output uses.
pub fn dotted(path: &str) -> String {
    if path.is_empty() || path.starts_with("./") || path.starts_with('/') || path == "." {
        path.to_string()
    } else {
        format!("./{path}")
    }
}

pub fn relative_path(root: &Path, path: &Path) -> PathBuf {
    let clean = |p: &Path| -> PathBuf {
        p.components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .collect()
    };
    let (root_clean, path_clean) = (clean(root), clean(path));
    if root_clean.as_os_str().is_empty() && path_clean.is_absolute() {
        return path.to_path_buf();
    }
    match path_clean.strip_prefix(&root_clean) {
        Ok(rel) => PathBuf::from(".").join(rel),
        Err(_) => path.to_path_buf(),
    }
}

/// A map with its keys in order.
fn serialize_sorted<S, K, V>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    K: Serialize + Ord,
    V: Serialize,
{
    let sorted: std::collections::BTreeMap<&K, &V> = map.iter().collect();
    sorted.serialize(serializer)
}

fn serialize_dep_graph<S>(
    map: &HashMap<PathBuf, Vec<PathBuf>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    // Sorted, so the same repository always gives the same output.
    let mut entries: Vec<(&PathBuf, &Vec<PathBuf>)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut m = serializer.serialize_map(Some(map.len()))?;
    for (k, v) in entries {
        let key = k.display().to_string();
        let vals: Vec<String> = v.iter().map(|p| p.display().to_string()).collect();
        m.serialize_entry(&key, &vals)?;
    }
    m.end()
}

fn deserialize_dep_graph<'de, D>(
    deserializer: D,
) -> Result<HashMap<PathBuf, Vec<PathBuf>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, Vec<String>> = HashMap::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| (PathBuf::from(k), v.into_iter().map(PathBuf::from).collect()))
        .collect())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn relative_paths_always_start_with_dot_slash() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            relative_path(Path::new("."), Path::new("src/a.rs")),
            p("./src/a.rs")
        );
        assert_eq!(
            relative_path(Path::new("."), Path::new("./src/a.rs")),
            p("./src/a.rs")
        );
        assert_eq!(
            relative_path(Path::new("/r"), Path::new("/r/src/a.rs")),
            p("./src/a.rs")
        );
        assert_eq!(
            relative_path(Path::new("./r"), Path::new("r/a.rs")),
            p("./a.rs")
        );
        assert_eq!(dotted("src/a.rs"), "./src/a.rs");
        assert_eq!(dotted("./src/a.rs"), "./src/a.rs");
    }

    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
    }

    // -----------------------------------------------------------------------
    // infer_project_type
    // -----------------------------------------------------------------------

    #[test]
    fn infers_full_stack_web_app() {
        let fw = vec!["React".to_string(), "Express".to_string()];
        let db = vec!["PostgreSQL".to_string()];
        let langs = HashMap::new();
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Full-stack web application"
        );
    }

    #[test]
    fn infers_frontend_only() {
        let fw = vec!["React".to_string(), "Vite".to_string()];
        let db = vec![];
        let langs = HashMap::new();
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Frontend web application"
        );
    }

    #[test]
    fn infers_rust_backend_service() {
        let fw = vec!["Axum".to_string(), "Tokio".to_string()];
        let db = vec!["PostgreSQL".to_string()];
        let mut langs = HashMap::new();
        langs.insert("Rust".to_string(), 5000usize);
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Rust backend service"
        );
    }

    #[test]
    fn infers_python_backend_service() {
        let fw = vec!["FastAPI".to_string()];
        let db = vec!["PostgreSQL".to_string()];
        let mut langs = HashMap::new();
        langs.insert("Python".to_string(), 3000usize);
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Python backend service"
        );
    }

    #[test]
    fn infers_blockchain_project() {
        let fw = vec![];
        let db = vec![];
        let mut langs = HashMap::new();
        langs.insert("Solidity".to_string(), 1000usize);
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Blockchain / Smart-contract project"
        );
    }

    #[test]
    fn infers_rust_library() {
        let fw = vec![];
        let db = vec![];
        let mut langs = HashMap::new();
        langs.insert("Rust".to_string(), 2000usize);
        let infra = vec![];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Rust library / CLI tool"
        );
    }

    #[test]
    fn infers_containerised_service_from_docker() {
        let fw = vec![];
        let db = vec![];
        let langs = HashMap::new();
        let infra = vec!["Docker".to_string()];
        assert_eq!(
            infer_project_type(&fw, &db, &langs, &infra),
            "Containerised service"
        );
    }

    // -----------------------------------------------------------------------
    // infer_architecture
    // -----------------------------------------------------------------------

    #[test]
    fn infers_frontend_api_database_architecture() {
        let fw = vec!["React".to_string(), "Express".to_string()];
        let db = vec!["PostgreSQL".to_string()];
        assert_eq!(infer_architecture(&fw, &db), "Frontend → API → Database");
    }

    #[test]
    fn infers_frontend_api_architecture() {
        let fw = vec!["Vue".to_string(), "Express".to_string()];
        let db = vec![];
        assert_eq!(infer_architecture(&fw, &db), "Frontend → API");
    }

    #[test]
    fn infers_api_database_architecture() {
        let fw = vec!["FastAPI".to_string()];
        let db = vec!["MongoDB".to_string()];
        assert_eq!(infer_architecture(&fw, &db), "API → Database");
    }

    #[test]
    fn infers_frontend_only_architecture() {
        let fw = vec!["React".to_string()];
        let db = vec![];
        assert_eq!(infer_architecture(&fw, &db), "Frontend only");
    }

    // -----------------------------------------------------------------------
    // analyze() — integration smoke test
    // -----------------------------------------------------------------------

    #[test]
    fn analyze_returns_populated_result_on_minimal_repo() {
        let root = unique_temp_dir("codedna_analysis_smoke");
        let src = root.join("src");

        // package.json with React + PostgreSQL
        write_file(
            &root.join("package.json"),
            r#"{ "dependencies": { "react": "^18.0.0", "pg": "^8.0.0" } }"#,
        );

        // Entry point
        write_file(
            &src.join("index.ts"),
            "import { App } from './App';\nconsole.log('hello');",
        );
        // Imported file
        write_file(&src.join("App.tsx"), "export const App = () => null;");
        // Dead file
        write_file(&src.join("unused.ts"), "export const old = true;");

        let result = analyze(&root);

        // Languages detected
        assert!(result.languages.contains_key("TypeScript"));
        assert!(result.total_loc > 0);

        // Frameworks / databases
        assert!(result.frameworks.contains(&"React".to_string()));
        assert!(result.databases.contains(&"PostgreSQL".to_string()));
        // infrastructure field is present (may be empty for this test repo)
        let _ = &result.infrastructure;

        // Architecture inferred (React + PostgreSQL, no backend framework → Frontend → Database)
        assert_eq!(result.architecture, "Frontend → Database");

        // File breakdown populated and sorted descending by LOC
        assert!(!result.file_breakdown.is_empty());
        let locs: Vec<usize> = result.file_breakdown.iter().map(|f| f.loc).collect();
        let mut sorted = locs.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(locs, sorted);

        // Dependency graph has entries
        assert!(!result.dependency_graph.is_empty());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn analyze_leaves_lockfiles_out_of_line_counts() {
        let root = unique_temp_dir("codedna_analysis_lockfile");
        write_file(&root.join("index.js"), "console.log('hello');");
        write_file(&root.join("package-lock.json"), "{\n  \"v\": 3\n}");
        write_file(&root.join("web").join("yarn.lock"), "# yarn lockfile v1");

        let result = analyze(&root);

        assert_eq!(result.total_loc, 1);
        assert!(!result.languages.contains_key("JSON"));
        assert_eq!(result.file_breakdown.len(), 1);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn analyze_handles_empty_directory() {
        let root = unique_temp_dir("codedna_analysis_empty");
        let result = analyze(&root);

        assert_eq!(result.total_loc, 0);
        assert!(result.frameworks.is_empty());
        assert!(result.databases.is_empty());
        assert!(result.dead_code.is_empty());
        assert!(result.file_breakdown.is_empty());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn report_states_its_schema_and_the_published_schema_is_current() {
        let root = std::env::temp_dir().join(format!("codedna_schema_{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let json = serde_json::to_value(analyze(&root)).unwrap();
        assert_eq!(json["schema_version"], SCHEMA_VERSION);
        assert_eq!(json["analyzer"]["name"], "codedna");
        assert_eq!(json["analyzer"]["version"], env!("CARGO_PKG_VERSION"));
        fs::remove_dir_all(root).ok();

        // Reports written before versioning still load.
        let old: AnalysisResult = serde_json::from_value({
            let mut v = json.clone();
            v.as_object_mut().unwrap().remove("schema_version");
            v.as_object_mut().unwrap().remove("analyzer");
            v
        })
        .unwrap();
        assert_eq!(old.schema_version, 0);

        let published = include_str!("../schema/report.schema.json");
        assert_eq!(
            published.replace("\r\n", "\n").trim_end(),
            json_schema(),
            "run: codedna schema > schema/report.schema.json"
        );
    }
}
