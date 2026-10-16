//! AI context pack.
//!
//! Condenses an `AnalysisResult` into a short briefing an AI coding agent can
//! read before touching a repository: what the project is, its stack, where
//! execution starts, which files matter most, and a trimmed directory map.
//!
//! The pack is kept within a token budget (estimated as four characters per
//! token) so it fits comfortably into an agent's context window.

use crate::analysis::AnalysisResult;
use crate::dead_code_detector::is_entry_point;
use crate::safe_text::sanitize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};

/// Default token budget for a context pack.
pub const DEFAULT_MAX_TOKENS: usize = 2000;

/// Smallest budget accepted; anything lower could not hold the summary.
pub const MIN_MAX_TOKENS: usize = 300;

/// Rough characters-per-token ratio used for the budget.
const CHARS_PER_TOKEN: usize = 4;

/// How many entries each ranked list keeps.
const LIST_LIMIT: usize = 10;

/// Packages listed per workspace.
const WORKSPACE_PACKAGE_LIMIT: usize = 20;

/// Depth of the directory map before it is trimmed to the budget.
pub const MAP_DEPTH: usize = 3;

/// A language and its share of the code.
#[derive(Debug, Serialize)]
pub struct LanguageShare {
    pub name: String,
    pub loc: usize,
    pub percent: usize,
}

/// A file and how many other files import it.
#[derive(Debug, Serialize)]
pub struct ImportedFile {
    pub file: String,
    pub imported_by: usize,
}

/// A file and its size.
/// A workspace and the first of its packages, each as
/// `name (path): 1.2k LOC · TypeScript · React · uses ui`.
#[derive(Debug, Serialize)]
pub struct WorkspaceLine {
    pub summary: String,
    pub packages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SizedFile {
    pub file: String,
    pub loc: usize,
    pub language: String,
}

/// The context pack.
#[derive(Debug, Serialize)]
pub struct ContextPack {
    pub project_type: String,
    pub architecture: String,
    /// Layers found in the code with a confidence level, or empty.
    pub code_layers: String,
    pub total_loc: usize,
    pub file_count: usize,
    pub languages: Vec<LanguageShare>,
    pub frameworks: Vec<String>,
    pub databases: Vec<String>,
    pub infrastructure: Vec<String>,
    pub entry_points: Vec<String>,
    pub most_imported: Vec<ImportedFile>,
    pub largest_files: Vec<SizedFile>,
    pub dead_code_candidates: usize,
    /// Candidates per confidence level, e.g. "2 high, 1 low".
    pub dead_code_confidence: String,
    /// Groups of files that import each other.
    pub circular_import_count: usize,
    /// The first few of those, each as `a → b → a`.
    pub circular_imports: Vec<String>,
    /// Workspaces with their packages, e.g. `Cargo (Cargo.toml): 2 packages`.
    pub workspaces: Vec<WorkspaceLine>,
    pub map: String,
    pub map_truncated: bool,
}

/// Build a context pack from an analysis and a rendered directory map.
///
/// `tree` is the output of `repo_map::render_tree`; its first line (the root
/// label, which for a cloned repository is a random temp-dir name) is
/// replaced with `./`.
pub fn build(result: &AnalysisResult, tree: &str, max_tokens: usize) -> ContextPack {
    let mut pack = ContextPack {
        project_type: result.project_type.clone(),
        architecture: result.architecture.clone(),
        code_layers: code_layers(result),
        total_loc: result.total_loc,
        file_count: result.file_breakdown.len(),
        languages: languages(result),
        frameworks: result.frameworks.clone(),
        databases: result.databases.clone(),
        infrastructure: result.infrastructure.clone(),
        entry_points: entry_points(result),
        most_imported: most_imported(result),
        largest_files: result
            .file_breakdown
            .iter()
            .take(LIST_LIMIT)
            .map(|f| SizedFile {
                file: show(&f.file),
                loc: f.loc,
                language: f.language.clone(),
            })
            .collect(),
        dead_code_candidates: result.dead_code.len(),
        dead_code_confidence: crate::dead_code_detector::confidence_summary(
            &result.dead_code_candidates(),
        ),
        circular_import_count: result.circular_imports.len(),
        circular_imports: result
            .circular_imports
            .iter()
            .take(LIST_LIMIT)
            .map(|c| short_cycle_line(c))
            .collect(),
        workspaces: result
            .workspaces
            .iter()
            .map(|w| WorkspaceLine {
                summary: crate::workspaces::summary(w),
                packages: w
                    .packages
                    .iter()
                    .take(WORKSPACE_PACKAGE_LIMIT)
                    .map(|p| {
                        format!(
                            "{} ({}): {}",
                            p.name,
                            show(&p.path),
                            crate::workspaces::package_line(p)
                        )
                    })
                    .collect(),
            })
            .collect(),
        map: String::new(),
        map_truncated: false,
    };

    // Everything except the map is fixed in size; the map gets what is left.
    let budget = max_tokens.max(MIN_MAX_TOKENS) * CHARS_PER_TOKEN;
    let used = render_markdown(&pack).chars().count();
    let room = budget.saturating_sub(used);

    let mut lines = tree.lines();
    lines.next();
    let mut map = String::from("./\n");
    for line in lines {
        if map.chars().count() + line.chars().count() + 1 > room {
            pack.map_truncated = true;
            break;
        }
        map.push_str(line);
        map.push('\n');
    }
    pack.map = map;
    pack
}

/// `"Routes → Services → Data (high confidence)"`, or empty when the code
/// has no recognisable layers.
fn code_layers(result: &AnalysisResult) -> String {
    let details = &result.architecture_details;
    details
        .layer_chain()
        .map(|chain| format!("{chain} ({} confidence)", details.confidence.as_str()))
        .unwrap_or_default()
}

/// Render a context pack as Markdown.
pub fn render_markdown(pack: &ContextPack) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Codebase context\n");
    let _ = writeln!(out, "- Project type: {}", pack.project_type);
    let _ = writeln!(out, "- Architecture: {}", pack.architecture);
    if !pack.code_layers.is_empty() {
        let _ = writeln!(out, "- Code layers: {}", pack.code_layers);
    }
    let _ = writeln!(
        out,
        "- Size: {} lines of code in {} files",
        pack.total_loc, pack.file_count
    );

    let _ = writeln!(out, "\n## Stack\n");
    let langs: Vec<String> = pack
        .languages
        .iter()
        .map(|l| format!("{} {}%", l.name, l.percent))
        .collect();
    let _ = writeln!(out, "- Languages: {}", list_or_none(&langs));
    let _ = writeln!(out, "- Frameworks: {}", list_or_none(&pack.frameworks));
    let _ = writeln!(out, "- Databases: {}", list_or_none(&pack.databases));
    let _ = writeln!(
        out,
        "- Infrastructure: {}",
        list_or_none(&pack.infrastructure)
    );

    if !pack.workspaces.is_empty() {
        let _ = writeln!(out, "\n## Workspaces\n");
        for w in &pack.workspaces {
            let _ = writeln!(out, "- {}", w.summary);
            for p in &w.packages {
                let _ = writeln!(out, "  - {p}");
            }
        }
    }

    if !pack.entry_points.is_empty() {
        let _ = writeln!(out, "\n## Entry points\n");
        for f in &pack.entry_points {
            let _ = writeln!(out, "- {}", code(f));
        }
    }

    if !pack.most_imported.is_empty() {
        let _ = writeln!(out, "\n## Most imported files\n");
        for f in &pack.most_imported {
            let _ = writeln!(out, "- {} (imported by {})", code(&f.file), f.imported_by);
        }
    }

    if !pack.largest_files.is_empty() {
        let _ = writeln!(out, "\n## Largest files\n");
        for f in &pack.largest_files {
            let _ = writeln!(out, "- {} ({} LOC, {})", code(&f.file), f.loc, f.language);
        }
    }

    if !pack.map.is_empty() {
        let _ = writeln!(out, "\n## Directory map\n");
        let _ = writeln!(out, "```text\n{}```", pack.map);
        if pack.map_truncated {
            let _ = writeln!(out, "(map trimmed to fit the token budget)");
        }
    }

    if pack.dead_code_candidates > 0 || pack.circular_import_count > 0 {
        let _ = writeln!(out, "\n## Notes\n");
    }
    if pack.dead_code_candidates > 0 {
        let _ = writeln!(
            out,
            "- {} file{} not reachable from a detected entry point ({}). This is a heuristic: \
             verify before deleting anything; `codedna deadcode` gives the reason for each.",
            pack.dead_code_candidates,
            if pack.dead_code_candidates == 1 {
                " is"
            } else {
                "s are"
            },
            pack.dead_code_confidence
        );
    }
    if pack.circular_import_count > 0 {
        let _ = writeln!(
            out,
            "- {} circular import{} (files that import each other), e.g. {}. \
             `codedna cycles` lists them.",
            pack.circular_import_count,
            if pack.circular_import_count == 1 {
                ""
            } else {
                "s"
            },
            code(
                pack.circular_imports
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
    }

    out
}

/// A cycle as `a → b → a`, with long cycles shortened to their first files.
pub fn short_cycle_line(cycle: &[PathBuf]) -> String {
    const SHOWN: usize = 6;
    let mut parts: Vec<String> = cycle.iter().take(SHOWN).map(|p| show(p)).collect();
    if cycle.len() > SHOWN {
        parts.push(format!("… ({} files)", cycle.len()));
    }
    parts.extend(cycle.first().map(|p| show(p)));
    parts.join(" → ")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn languages(result: &AnalysisResult) -> Vec<LanguageShare> {
    let mut langs: Vec<LanguageShare> = result
        .languages
        .iter()
        .map(|(name, loc)| LanguageShare {
            name: name.clone(),
            loc: *loc,
            percent: (loc * 100).checked_div(result.total_loc).unwrap_or(0),
        })
        .collect();
    langs.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.name.cmp(&b.name)));
    langs
}

fn entry_points(result: &AnalysisResult) -> Vec<String> {
    let mut files: Vec<&PathBuf> = result
        .file_breakdown
        .iter()
        .map(|f| &f.file)
        .filter(|f| is_entry_point(f))
        .collect();
    // Shallow files first: `./src/main.rs` matters more than a nested fixture.
    files.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    files
        .into_iter()
        .take(LIST_LIMIT)
        .map(|f| show(f.as_path()))
        .collect()
}

fn most_imported(result: &AnalysisResult) -> Vec<ImportedFile> {
    let mut counts: HashMap<&PathBuf, usize> = HashMap::new();
    for (from, targets) in &result.dependency_graph {
        let unique: HashSet<&PathBuf> = targets.iter().filter(|t| *t != from).collect();
        for target in unique {
            *counts.entry(target).or_insert(0) += 1;
        }
    }
    let mut ranked: Vec<(&PathBuf, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(LIST_LIMIT)
        .map(|(file, imported_by)| ImportedFile {
            file: show(file),
            imported_by,
        })
        .collect()
}

fn show(path: &Path) -> String {
    sanitize(&path.to_string_lossy())
}

fn code(s: &str) -> String {
    format!("`{}`", s.replace('`', "'"))
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none detected".to_string()
    } else {
        items.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;

    fn sample() -> AnalysisResult {
        let mut languages = HashMap::new();
        languages.insert("Rust".to_string(), 900);
        languages.insert("TOML".to_string(), 100);

        let mut graph = HashMap::new();
        graph.insert(
            PathBuf::from("./src/main.rs"),
            vec![
                PathBuf::from("./src/cli.rs"),
                PathBuf::from("./src/util.rs"),
            ],
        );
        graph.insert(
            PathBuf::from("./src/cli.rs"),
            vec![PathBuf::from("./src/util.rs")],
        );
        graph.insert(PathBuf::from("./src/util.rs"), vec![]);

        AnalysisResult {
            project_type: "Rust library / CLI tool".to_string(),
            total_loc: 1000,
            comment_lines: 0,
            blank_lines: 0,
            architecture_details: Default::default(),
            dead_code_details: Vec::new(),
            circular_imports: Vec::new(),
            workspaces: Vec::new(),
            languages,
            frameworks: vec![],
            databases: vec![],
            infrastructure: vec!["GitHub Actions".to_string()],
            architecture: "Monolithic / undetermined".to_string(),
            dead_code: vec![PathBuf::from("./src/old.rs")],
            dependency_graph: graph,
            file_breakdown: vec![
                FileInfo {
                    file: PathBuf::from("./src/cli.rs"),
                    loc: 500,
                    language: "Rust".to_string(),
                },
                FileInfo {
                    file: PathBuf::from("./src/main.rs"),
                    loc: 300,
                    language: "Rust".to_string(),
                },
                FileInfo {
                    file: PathBuf::from("./src/util.rs"),
                    loc: 100,
                    language: "Rust".to_string(),
                },
            ],
        }
    }

    const TREE: &str = "tmpXYZ/\n├── src/\n│   ├── cli.rs\n│   └── main.rs\n└── Cargo.toml\n";

    #[test]
    fn ranks_languages_and_imports() {
        let pack = build(&sample(), TREE, DEFAULT_MAX_TOKENS);
        assert_eq!(pack.languages[0].name, "Rust");
        assert_eq!(pack.languages[0].percent, 90);
        assert_eq!(pack.most_imported[0].file, "./src/util.rs");
        assert_eq!(pack.most_imported[0].imported_by, 2);
        assert_eq!(pack.file_count, 3);
        assert_eq!(pack.dead_code_candidates, 1);
    }

    #[test]
    fn finds_entry_points() {
        let pack = build(&sample(), TREE, DEFAULT_MAX_TOKENS);
        assert!(pack.entry_points.contains(&"./src/main.rs".to_string()));
        assert!(pack.entry_points.contains(&"./src/cli.rs".to_string()));
        assert!(!pack.entry_points.contains(&"./src/util.rs".to_string()));
    }

    #[test]
    fn replaces_root_label_in_map() {
        let pack = build(&sample(), TREE, DEFAULT_MAX_TOKENS);
        assert!(pack.map.starts_with("./\n"));
        assert!(!pack.map.contains("tmpXYZ"));
        assert!(!pack.map_truncated);
    }

    #[test]
    fn stays_within_budget() {
        let mut tree = String::from("root/\n");
        for i in 0..5000 {
            tree.push_str(&format!("├── file_{i}.rs\n"));
        }
        let pack = build(&sample(), &tree, MIN_MAX_TOKENS);
        assert!(pack.map_truncated);
        let md = render_markdown(&pack);
        // A little slack for the trimmed-map notice.
        assert!(md.chars().count() <= MIN_MAX_TOKENS * CHARS_PER_TOKEN + 100);
    }

    #[test]
    fn markdown_has_sections() {
        let md = render_markdown(&build(&sample(), TREE, DEFAULT_MAX_TOKENS));
        assert!(md.contains("# Codebase context"));
        assert!(md.contains("- Languages: Rust 90%, TOML 10%"));
        assert!(md.contains("## Entry points"));
        assert!(md.contains("## Directory map"));
        assert!(md.contains("verify before deleting"));
    }
}
