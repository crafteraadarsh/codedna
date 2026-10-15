//! Compare two analyses of a codebase.
//!
//! Either side can be a saved snapshot (the output of `codedna json`) or a
//! folder or repository analysed on the spot. The diff shows what changed in
//! the stack, languages, size, files and dead-code candidates.
//!
//! Snapshot files are untrusted input: their size is capped, and every name
//! read from them is sanitized before it is shown.

use crate::analysis::AnalysisResult;
use crate::safe_text::sanitize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// Largest snapshot file read, in bytes.
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
/// Most files listed in each of added, removed and changed.
pub const MAX_FILES: usize = 25;

/// A value that changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub before: String,
    pub after: String,
}

/// Names that appeared or disappeared.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NameDelta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl NameDelta {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// Lines of code in one language, before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanguageChange {
    pub language: String,
    pub before: usize,
    pub after: usize,
}

/// A file present on one side only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileLoc {
    pub file: String,
    pub loc: usize,
}

/// A file present on both sides whose size changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
    pub file: String,
    pub before: usize,
    pub after: usize,
}

/// Everything that differs between two analyses.
#[derive(Debug, Clone, Serialize)]
pub struct Diff {
    pub loc_before: usize,
    pub loc_after: usize,
    pub files_before: usize,
    pub files_after: usize,
    pub project_type: Option<Change>,
    pub architecture: Option<Change>,
    pub frameworks: NameDelta,
    pub databases: NameDelta,
    pub infrastructure: NameDelta,
    /// Languages whose line count changed, largest change first.
    pub languages: Vec<LanguageChange>,
    /// Totals before the file lists below were capped at `MAX_FILES`.
    pub files_added_total: usize,
    pub files_removed_total: usize,
    pub files_changed_total: usize,
    /// Largest first.
    pub files_added: Vec<FileLoc>,
    pub files_removed: Vec<FileLoc>,
    /// Largest change first.
    pub files_changed: Vec<FileChange>,
    /// Files that are dead-code candidates now but were not before.
    pub new_dead_code: Vec<String>,
}

impl Diff {
    /// Whether nothing at all changed.
    pub fn is_empty(&self) -> bool {
        self.loc_before == self.loc_after
            && self.files_before == self.files_after
            && self.project_type.is_none()
            && self.architecture.is_none()
            && self.frameworks.is_empty()
            && self.databases.is_empty()
            && self.infrastructure.is_empty()
            && self.languages.is_empty()
            && self.files_added_total == 0
            && self.files_removed_total == 0
            && self.files_changed_total == 0
            && self.new_dead_code.is_empty()
    }
}

/// Read a snapshot saved with `codedna json`.
pub fn load_snapshot(path: &Path) -> Result<AnalysisResult, String> {
    let shown = sanitize(&path.display().to_string());
    let size = std::fs::metadata(path)
        .map_err(|_| format!("Could not read {shown}."))?
        .len();
    if size > MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "{shown} is larger than {} MB, so it was not read.",
            MAX_SNAPSHOT_BYTES / (1024 * 1024)
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|_| format!("Could not read {shown}."))?;
    serde_json::from_str(&text)
        .map_err(|_| format!("{shown} is not a snapshot saved with `codedna json`."))
}

/// Whether a CLI argument names a saved snapshot rather than a folder or URL.
pub fn is_snapshot(input: &str) -> bool {
    input.to_ascii_lowercase().ends_with(".json") && Path::new(input).is_file()
}

/// Compare `before` with `after`.
pub fn diff(before: &AnalysisResult, after: &AnalysisResult) -> Diff {
    let old = sizes(before);
    let new = sizes(after);

    let mut added: Vec<FileLoc> = new
        .iter()
        .filter(|(f, _)| !old.contains_key(*f))
        .map(|(f, &loc)| FileLoc {
            file: f.clone(),
            loc,
        })
        .collect();
    added.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.file.cmp(&b.file)));
    let mut removed: Vec<FileLoc> = old
        .iter()
        .filter(|(f, _)| !new.contains_key(*f))
        .map(|(f, &loc)| FileLoc {
            file: f.clone(),
            loc,
        })
        .collect();
    removed.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.file.cmp(&b.file)));
    let mut changed: Vec<FileChange> = new
        .iter()
        .filter_map(|(f, &after)| {
            let before = *old.get(f)?;
            (before != after).then(|| FileChange {
                file: f.clone(),
                before,
                after,
            })
        })
        .collect();
    changed.sort_by(|a, b| {
        a.before
            .abs_diff(a.after)
            .cmp(&b.before.abs_diff(b.after))
            .reverse()
            .then_with(|| a.file.cmp(&b.file))
    });

    let was_dead: BTreeSet<String> = before.dead_code.iter().map(|p| show(p)).collect();
    let mut new_dead_code: Vec<String> = after
        .dead_code
        .iter()
        .map(|p| show(p))
        .filter(|f| !was_dead.contains(f))
        .collect();
    new_dead_code.sort();
    new_dead_code.dedup();
    new_dead_code.truncate(MAX_FILES);

    let (files_added_total, files_removed_total, files_changed_total) =
        (added.len(), removed.len(), changed.len());
    added.truncate(MAX_FILES);
    removed.truncate(MAX_FILES);
    changed.truncate(MAX_FILES);

    Diff {
        loc_before: before.total_loc,
        loc_after: after.total_loc,
        files_before: before.file_breakdown.len(),
        files_after: after.file_breakdown.len(),
        project_type: change(&before.project_type, &after.project_type),
        architecture: change(&before.architecture, &after.architecture),
        frameworks: names(&before.frameworks, &after.frameworks),
        databases: names(&before.databases, &after.databases),
        infrastructure: names(&before.infrastructure, &after.infrastructure),
        languages: languages(&before.languages, &after.languages),
        files_added_total,
        files_removed_total,
        files_changed_total,
        files_added: added,
        files_removed: removed,
        files_changed: changed,
        new_dead_code,
    }
}

/// File sizes keyed by display path. Paths are compared without the
/// leading `./` so snapshots of the same repository line up.
fn sizes(result: &AnalysisResult) -> BTreeMap<String, usize> {
    result
        .file_breakdown
        .iter()
        .map(|f| (show(&f.file), f.loc))
        .collect()
}

fn show(path: &Path) -> String {
    let path = path.strip_prefix(".").unwrap_or(path);
    sanitize(&path.to_string_lossy().replace('\\', "/"))
}

fn change(before: &str, after: &str) -> Option<Change> {
    (before != after).then(|| Change {
        before: sanitize(before),
        after: sanitize(after),
    })
}

fn names(before: &[String], after: &[String]) -> NameDelta {
    let old: BTreeSet<String> = before.iter().map(|s| sanitize(s)).collect();
    let new: BTreeSet<String> = after.iter().map(|s| sanitize(s)).collect();
    NameDelta {
        added: new.difference(&old).cloned().collect(),
        removed: old.difference(&new).cloned().collect(),
    }
}

fn languages(
    before: &HashMap<String, usize>,
    after: &HashMap<String, usize>,
) -> Vec<LanguageChange> {
    let all: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut out: Vec<LanguageChange> = all
        .into_iter()
        .map(|lang| LanguageChange {
            language: sanitize(lang),
            before: before.get(lang).copied().unwrap_or(0),
            after: after.get(lang).copied().unwrap_or(0),
        })
        .filter(|c| c.before != c.after)
        .collect();
    out.sort_by(|a, b| {
        a.before
            .abs_diff(a.after)
            .cmp(&b.before.abs_diff(b.after))
            .reverse()
            .then_with(|| a.language.cmp(&b.language))
    });
    out
}

fn signed(before: usize, after: usize) -> String {
    if after >= before {
        format!("+{}", after - before)
    } else {
        format!("-{}", before - after)
    }
}

/// Plain text for the terminal.
pub fn render_text(d: &Diff) -> String {
    let mut out = String::from("\n");
    if d.is_empty() {
        out.push_str("  No differences found.\n\n");
        return out;
    }
    out.push_str(&format!(
        "  Lines of code: {} → {} ({})\n",
        d.loc_before,
        d.loc_after,
        signed(d.loc_before, d.loc_after)
    ));
    out.push_str(&format!(
        "  Source files:  {} → {} ({})\n",
        d.files_before,
        d.files_after,
        signed(d.files_before, d.files_after)
    ));
    if let Some(c) = &d.project_type {
        out.push_str(&format!("  Project type:  {} → {}\n", c.before, c.after));
    }
    if let Some(c) = &d.architecture {
        out.push_str(&format!("  Architecture:  {} → {}\n", c.before, c.after));
    }

    for (label, delta) in [
        ("Frameworks", &d.frameworks),
        ("Databases", &d.databases),
        ("Infrastructure", &d.infrastructure),
    ] {
        if delta.is_empty() {
            continue;
        }
        out.push_str(&format!("\n  {label}\n"));
        for name in &delta.added {
            out.push_str(&format!("    + {name}\n"));
        }
        for name in &delta.removed {
            out.push_str(&format!("    - {name}\n"));
        }
    }

    if !d.languages.is_empty() {
        out.push_str("\n  Languages (lines of code)\n");
        for l in &d.languages {
            out.push_str(&format!(
                "    {:<16} {:>8} → {:<8} ({})\n",
                l.language,
                l.before,
                l.after,
                signed(l.before, l.after)
            ));
        }
    }

    let more = |shown: usize, total: usize| {
        if total > shown {
            format!("    … and {} more\n", total - shown)
        } else {
            String::new()
        }
    };
    if !d.files_added.is_empty() {
        out.push_str(&format!("\n  Files added ({})\n", d.files_added_total));
        for f in &d.files_added {
            out.push_str(&format!("    + {} ({} lines)\n", f.file, f.loc));
        }
        out.push_str(&more(d.files_added.len(), d.files_added_total));
    }
    if !d.files_removed.is_empty() {
        out.push_str(&format!("\n  Files removed ({})\n", d.files_removed_total));
        for f in &d.files_removed {
            out.push_str(&format!("    - {} ({} lines)\n", f.file, f.loc));
        }
        out.push_str(&more(d.files_removed.len(), d.files_removed_total));
    }
    if !d.files_changed.is_empty() {
        out.push_str(&format!(
            "\n  Files that changed size ({})\n",
            d.files_changed_total
        ));
        for f in &d.files_changed {
            out.push_str(&format!(
                "    {} {} → {} ({})\n",
                f.file,
                f.before,
                f.after,
                signed(f.before, f.after)
            ));
        }
        out.push_str(&more(d.files_changed.len(), d.files_changed_total));
    }
    if !d.new_dead_code.is_empty() {
        out.push_str("\n  New possibly unused files (heuristic: check before deleting)\n");
        for f in &d.new_dead_code {
            out.push_str(&format!("    {f}\n"));
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use std::path::PathBuf;

    fn result(files: &[(&str, usize)], frameworks: &[&str]) -> AnalysisResult {
        let mut languages = HashMap::new();
        for (_, loc) in files {
            *languages.entry("Rust".to_string()).or_insert(0) += loc;
        }
        AnalysisResult {
            project_type: "CLI tool".to_string(),
            total_loc: files.iter().map(|f| f.1).sum(),
            comment_lines: 0,
            blank_lines: 0,
            languages,
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            databases: vec![],
            infrastructure: vec![],
            architecture: "CLI".to_string(),
            dead_code: vec![],
            dependency_graph: HashMap::new(),
            file_breakdown: files
                .iter()
                .map(|(f, loc)| FileInfo {
                    file: PathBuf::from(f),
                    loc: *loc,
                    language: "Rust".to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn same_analysis_has_no_differences() {
        let a = result(&[("./a.rs", 10)], &["Axum"]);
        let d = diff(&a, &a.clone());
        assert!(d.is_empty());
        assert!(render_text(&d).contains("No differences"));
    }

    #[test]
    fn finds_added_removed_and_changed_files() {
        let before = result(&[("./a.rs", 10), ("./gone.rs", 5)], &["Axum"]);
        let mut after = result(&[("a.rs", 40), ("./new.rs", 7)], &["Axum", "Tokio"]);
        after.dead_code = vec![PathBuf::from("./new.rs")];

        let d = diff(&before, &after);
        assert_eq!(
            d.files_added,
            [FileLoc {
                file: "new.rs".into(),
                loc: 7
            }]
        );
        assert_eq!(
            d.files_removed,
            [FileLoc {
                file: "gone.rs".into(),
                loc: 5
            }]
        );
        assert_eq!(
            d.files_changed,
            [FileChange {
                file: "a.rs".into(),
                before: 10,
                after: 40
            }]
        );
        assert_eq!(d.frameworks.added, ["Tokio"]);
        assert!(d.frameworks.removed.is_empty());
        assert_eq!(d.new_dead_code, ["new.rs"]);
        assert_eq!(
            d.languages,
            [LanguageChange {
                language: "Rust".into(),
                before: 15,
                after: 47
            }]
        );

        let text = render_text(&d);
        assert!(text.contains("Lines of code: 15 → 47 (+32)"));
        assert!(text.contains("+ Tokio"));
        assert!(text.contains("- gone.rs (5 lines)"));
    }

    #[test]
    fn caps_long_file_lists() {
        let many: Vec<(String, usize)> = (0..MAX_FILES + 5)
            .map(|i| (format!("f{i}.rs"), i + 1))
            .collect();
        let refs: Vec<(&str, usize)> = many.iter().map(|(f, l)| (f.as_str(), *l)).collect();
        let d = diff(&result(&[], &[]), &result(&refs, &[]));
        assert_eq!(d.files_added.len(), MAX_FILES);
        assert_eq!(d.files_added_total, MAX_FILES + 5);
        assert!(render_text(&d).contains("… and 5 more"));
    }

    #[test]
    fn sanitizes_names_from_snapshots() {
        let before = result(&[], &[]);
        let after = result(&[("./a\u{1b}[2J.rs", 3)], &["X\u{202E}Y"]);
        let d = diff(&before, &after);
        assert_eq!(d.files_added[0].file, "a?[2J.rs");
        assert_eq!(d.frameworks.added, ["X?Y"]);
    }

    #[test]
    fn loads_saved_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("before.json");
        let a = result(&[("./a.rs", 10)], &["Axum"]);
        std::fs::write(&path, serde_json::to_string(&a).unwrap()).unwrap();
        let loaded = load_snapshot(&path).unwrap();
        assert_eq!(loaded.total_loc, 10);
        assert!(is_snapshot(path.to_str().unwrap()));

        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{\"not\": \"a snapshot\"}").unwrap();
        assert!(load_snapshot(&bad).unwrap_err().contains("not a snapshot"));
        assert!(!is_snapshot(dir.path().to_str().unwrap()));
    }
}
