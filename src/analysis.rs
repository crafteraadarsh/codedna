//! Analysis aggregation module.
//!
//! Wires every analysis module together and produces a single `AnalysisResult`
//! that represents the complete intelligence report for a repository.

use crate::dead_code_detector;
use crate::dependency_graph;
use crate::framework_detector;
use crate::language_detector;
use crate::loc_counter;
use crate::scanner;
use rayon::prelude::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Per-file metadata produced by the analysis pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    /// Path to the file (relative to the scanned root where possible).
    pub file: PathBuf,
    /// Non-empty lines of code counted in the file.
    pub loc: usize,
    /// Detected language name (e.g. "TypeScript", "Python").
    pub language: String,
}

/// Aggregated intelligence report for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// High-level project classification (e.g. "Full-stack web application").
    pub project_type: String,

    /// Total non-empty lines of code across all recognised source files.
    pub total_loc: usize,

    /// LOC per language: `{ "TypeScript": 12450, "Python": 800, … }`.
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

    /// Files unreachable from any entry point via dependency graph traversal.
    pub dead_code: Vec<PathBuf>,

    /// Directed import graph: `file → [files it imports]`.
    #[serde(serialize_with = "serialize_dep_graph")]
    #[serde(deserialize_with = "deserialize_dep_graph")]
    pub dependency_graph: HashMap<PathBuf, Vec<PathBuf>>,

    /// Per-file breakdown, sorted by LOC descending.
    pub file_breakdown: Vec<FileInfo>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the full analysis pipeline on `root` and return an `AnalysisResult`.
///
/// # Pipeline steps
/// 1. Scan repository files (respecting ignore rules)
/// 2. Count LOC per file; skip binary / unreadable files
/// 3. Detect language per file; build aggregated language LOC map
/// 4. Detect frameworks and databases from manifest files
/// 5. Build dependency graph from import/require statements
/// 6. Detect dead code via BFS from entry points
/// 7. Infer project type and architecture from collected signals
pub fn analyze(root: &Path) -> AnalysisResult {
    // ── 1. Scan ──────────────────────────────────────────────────────────────
    let files = scanner::scan_repository(root);

    // ── 2 & 3. LOC + language (parallel) ────────────────────────────────────
    // Each file is processed independently — ideal for rayon par_iter.
    let per_file: Vec<(PathBuf, usize, String)> = files
        .par_iter()
        .filter_map(|file| {
            let loc = loc_counter::count_lines(file)?;
            let language = language_detector::detect_language(file)
                .map(|l| l.to_string())
                .unwrap_or_else(|| "Other".to_string());
            Some((file.clone(), loc, language))
        })
        .collect();

    let mut file_loc_pairs: Vec<(PathBuf, usize)> = Vec::with_capacity(per_file.len());
    let mut file_breakdown: Vec<FileInfo> = Vec::with_capacity(per_file.len());

    for (file, loc, language) in per_file {
        file_loc_pairs.push((file.clone(), loc));
        file_breakdown.push(FileInfo {
            file,
            loc,
            language,
        });
    }

    // Sort file breakdown by LOC descending for at-a-glance readability.
    file_breakdown.sort_unstable_by(|a, b| b.loc.cmp(&a.loc));

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
    let dep_graph = dependency_graph::build_dependency_graph(&files);

    // ── 6. Dead code ──────────────────────────────────────────────────────────
    let dead_code = dead_code_detector::detect_dead_code(&dep_graph);

    // ── 7. Inference ──────────────────────────────────────────────────────────
    let project_type = infer_project_type(
        &fw_result.frameworks,
        &fw_result.databases,
        &languages,
        &infrastructure,
    );
    let architecture = infer_architecture(&fw_result.frameworks, &fw_result.databases);

    // ── 8. Normalise paths ──────────────────────────────────────────────────
    // Strip the `root` prefix so output is always relative, regardless of
    // whether the root was a local `.` or an absolute temp-dir path.
    let make_relative = |p: &Path| -> PathBuf {
        p.strip_prefix(root)
            .map(|rel| PathBuf::from(".").join(rel))
            .unwrap_or_else(|_| p.to_path_buf())
    };

    let dead_code = dead_code.into_iter().map(|p| make_relative(&p)).collect();

    let dependency_graph: HashMap<PathBuf, Vec<PathBuf>> = dep_graph
        .into_iter()
        .map(|(k, vs)| {
            let key = make_relative(&k);
            let vals = vs.iter().map(|v| make_relative(v)).collect();
            (key, vals)
        })
        .collect();

    let file_breakdown = file_breakdown
        .into_iter()
        .map(|fi| FileInfo {
            file: make_relative(&fi.file),
            loc: fi.loc,
            language: fi.language,
        })
        .collect();

    AnalysisResult {
        project_type,
        total_loc,
        languages,
        frameworks: fw_result.frameworks,
        databases: fw_result.databases,
        infrastructure,
        architecture,
        dead_code,
        dependency_graph,
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

fn serialize_dep_graph<S>(
    map: &HashMap<PathBuf, Vec<PathBuf>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut m = serializer.serialize_map(Some(map.len()))?;
    for (k, v) in map {
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
}
