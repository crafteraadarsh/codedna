//! Hotspot detection.
//!
//! A hotspot is a file that is both large and changed often: the number of
//! commits that touched it in a time window multiplied by its lines of code.
//! Bugs and refactoring effort tend to gather in these files.
//!
//! The same history gives co-change coupling: pairs of files that are
//! usually changed in the same commit, often a sign of a hidden dependency.
//!
//! History is read with `git2` tree-to-tree diffs only: nothing is checked
//! out and no hooks, filters or external diff drivers run, so nothing from
//! the repository is executed.

use crate::analysis::AnalysisResult;
use crate::safe_text::sanitize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default time window in days.
pub const DEFAULT_DAYS: u64 = 365;
/// Default number of hotspots shown.
pub const DEFAULT_LIMIT: usize = 15;
/// Upper bound on commits read, so very long histories stay fast.
pub const MAX_COMMITS: usize = 5_000;
/// History depth fetched when a remote repository is cloned for hotspots.
pub const CLONE_DEPTH: i32 = 1_000;
/// Number of coupled file pairs shown.
pub const COUPLING_LIMIT: usize = 10;
/// Commits changing more files than this are skipped for coupling: bulk
/// renames and formatting passes would otherwise pair everything.
pub const MAX_FILES_PER_COMMIT: usize = 30;
/// A pair needs at least this many shared commits to count as coupled.
pub const MIN_SHARED: usize = 3;
/// …and must share at least this percentage of the less active file's commits.
pub const MIN_PERCENT: usize = 50;

/// One ranked file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hotspot {
    /// Path relative to the analysed folder.
    pub file: String,
    /// Commits in the window that changed the file.
    pub commits: usize,
    /// Non-empty lines of code.
    pub loc: usize,
    /// `commits × loc`.
    pub score: usize,
}

/// Two files that are usually changed in the same commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Coupling {
    pub file: String,
    pub other: String,
    /// Commits in the window that changed both files.
    pub shared: usize,
    /// `shared` as a percentage of the commits that changed the less active file.
    pub percent: usize,
}

/// Ranked hotspots plus how much history they were based on.
#[derive(Debug, Clone, Serialize)]
pub struct HotspotReport {
    /// Time window in days.
    pub days: u64,
    /// Commits read inside the window.
    pub commits_scanned: usize,
    /// `true` when the walk stopped at `MAX_COMMITS`.
    pub truncated: bool,
    pub hotspots: Vec<Hotspot>,
    /// Source files that usually change together, most shared commits first.
    pub coupling: Vec<Coupling>,
}

/// Commit counts per file, relative to the analysed folder.
#[derive(Debug, Default)]
pub struct Churn {
    pub counts: HashMap<PathBuf, usize>,
    /// Commits that changed both files of a pair; the pair is sorted.
    pub pairs: HashMap<(PathBuf, PathBuf), usize>,
    pub commits: usize,
    pub truncated: bool,
}

/// Count how many commits in the last `days` days changed each file under
/// `root`. Merge commits and the first commit are not counted as changes.
/// Renames are not followed.
pub fn churn(root: &Path, days: u64) -> Result<Churn, String> {
    let no_history = || "No Git history found here: hotspots need a Git repository.".to_string();
    let repo = git2::Repository::discover(root).map_err(|_| no_history())?;
    let workdir = repo
        .workdir()
        .and_then(|w| w.canonicalize().ok())
        .ok_or_else(no_history)?;
    let root = root.canonicalize().map_err(|_| no_history())?;
    let prefix = root
        .strip_prefix(&workdir)
        .map_err(|_| no_history())?
        .to_path_buf();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let window = i64::try_from(days.saturating_mul(86_400)).unwrap_or(i64::MAX);
    let cutoff = now.saturating_sub(window);

    let mut walk = repo.revwalk().map_err(|_| no_history())?;
    if walk.push_head().is_err() {
        return Err("This repository has no commits yet.".to_string());
    }
    walk.set_sorting(git2::Sort::TIME)
        .map_err(|_| no_history())?;

    let mut churn = Churn::default();
    for oid in walk {
        let Ok(commit) = oid.and_then(|oid| repo.find_commit(oid)) else {
            break;
        };
        if commit.time().seconds() < cutoff {
            break;
        }
        if churn.commits == MAX_COMMITS {
            churn.truncated = true;
            break;
        }
        churn.commits += 1;

        if commit.parent_count() != 1 {
            continue;
        }
        let (Ok(parent), Ok(new_tree)) = (commit.parent(0), commit.tree()) else {
            continue;
        };
        let Ok(old_tree) = parent.tree() else {
            continue;
        };
        let Ok(diff) = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None) else {
            continue;
        };
        let mut changed: Vec<PathBuf> = diff
            .deltas()
            .filter_map(|delta| delta.new_file().path())
            .filter_map(|path| path.strip_prefix(&prefix).ok())
            .map(Path::to_path_buf)
            .collect();
        changed.sort();
        changed.dedup();
        for path in &changed {
            *churn.counts.entry(path.clone()).or_insert(0) += 1;
        }
        if changed.len() <= MAX_FILES_PER_COMMIT {
            for (i, a) in changed.iter().enumerate() {
                for b in &changed[i + 1..] {
                    *churn.pairs.entry((a.clone(), b.clone())).or_insert(0) += 1;
                }
            }
        }
    }
    Ok(churn)
}

/// Rank the analysed source files by `commits × loc`, highest first.
pub fn rank(result: &AnalysisResult, churn: &Churn, limit: usize) -> Vec<Hotspot> {
    let mut hotspots: Vec<Hotspot> = result
        .file_breakdown
        .iter()
        .filter_map(|f| {
            // Analysis paths start with "./"; Git paths do not.
            let path = f.file.strip_prefix(".").unwrap_or(&f.file);
            let commits = *churn.counts.get(path)?;
            Some(Hotspot {
                file: show(path),
                commits,
                loc: f.loc,
                score: commits.saturating_mul(f.loc),
            })
        })
        .filter(|h| h.score > 0)
        .collect();
    hotspots.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.file.cmp(&b.file)));
    hotspots.truncate(limit);
    hotspots
}

/// Source-file pairs from `result` that usually change together, most shared
/// commits first.
pub fn couple(result: &AnalysisResult, churn: &Churn, limit: usize) -> Vec<Coupling> {
    let source: HashSet<&Path> = result
        .file_breakdown
        .iter()
        .map(|f| f.file.strip_prefix(".").unwrap_or(&f.file))
        .collect();
    let mut pairs: Vec<Coupling> = churn
        .pairs
        .iter()
        .filter(|((a, b), shared)| {
            **shared >= MIN_SHARED && source.contains(a.as_path()) && source.contains(b.as_path())
        })
        .filter_map(|((a, b), &shared)| {
            let fewer = (*churn.counts.get(a)?).min(*churn.counts.get(b)?).max(1);
            let percent = shared.saturating_mul(100) / fewer;
            (percent >= MIN_PERCENT).then(|| Coupling {
                file: show(a),
                other: show(b),
                shared,
                percent,
            })
        })
        .collect();
    pairs.sort_by(|x, y| {
        y.shared
            .cmp(&x.shared)
            .then(y.percent.cmp(&x.percent))
            .then_with(|| x.file.cmp(&y.file))
            .then_with(|| x.other.cmp(&y.other))
    });
    pairs.truncate(limit);
    pairs
}

fn show(path: &Path) -> String {
    crate::analysis::dotted(&sanitize(&path.to_string_lossy().replace('\\', "/")))
}

/// Read history under `root` and rank the files in `result`.
pub fn analyze(
    root: &Path,
    result: &AnalysisResult,
    days: u64,
    limit: usize,
) -> Result<HotspotReport, String> {
    let churn = churn(root, days)?;
    Ok(HotspotReport {
        days,
        commits_scanned: churn.commits,
        truncated: churn.truncated,
        hotspots: rank(result, &churn, limit),
        coupling: couple(result, &churn, COUPLING_LIMIT),
    })
}

/// Plain-text table for the terminal.
pub fn render_text(report: &HotspotReport) -> String {
    let mut out = String::from("\n");
    out.push_str(&format!(
        "  Hotspots: last {} days, {} commit{}{}\n\n",
        report.days,
        report.commits_scanned,
        if report.commits_scanned == 1 { "" } else { "s" },
        if report.truncated {
            format!(" (stopped at the newest {MAX_COMMITS})")
        } else {
            String::new()
        }
    ));
    if report.hotspots.is_empty() {
        out.push_str("  No source file changed in this window.\n\n");
        return out;
    }
    out.push_str(&format!(
        "  {:>4}  {:>7}  {:>8}  File\n",
        "Rank", "Commits", "LOC"
    ));
    out.push_str(&format!("  {}\n", "─".repeat(40)));
    for (i, h) in report.hotspots.iter().enumerate() {
        out.push_str(&format!(
            "  {:>4}  {:>7}  {:>8}  {}\n",
            i + 1,
            h.commits,
            h.loc,
            h.file
        ));
    }
    out.push_str("\n  Ranked by commits × lines of code. Large files that change often\n");
    out.push_str("  are where bugs and refactoring effort usually gather.\n\n");
    if !report.coupling.is_empty() {
        out.push_str("  Files that change together\n\n");
        out.push_str(&format!("  {:>6}  {:>8}  Files\n", "Shared", "Together"));
        out.push_str(&format!("  {}\n", "─".repeat(40)));
        for c in &report.coupling {
            out.push_str(&format!(
                "  {:>6}  {:>7}%  {} + {}\n",
                c.shared, c.percent, c.file, c.other
            ));
        }
        out.push_str("\n  Shared commits, and how often the less active file changed with\n");
        out.push_str("  the other. A change to one usually needs a change to both.\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;
    use git2::{Repository, Signature, Time};
    use std::fs;

    const DAY: i64 = 86_400;

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn commit(repo: &Repository, files: &[(&str, &str)], days_ago: i64) {
        let workdir = repo.workdir().unwrap().to_path_buf();
        let mut index = repo.index().unwrap();
        for (name, body) in files {
            let path = workdir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, body).unwrap();
            index.add_path(Path::new(name)).unwrap();
        }
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig =
            Signature::new("t", "t@example.com", &Time::new(now() - days_ago * DAY, 0)).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
            .unwrap();
    }

    fn result(files: &[(&str, usize)]) -> AnalysisResult {
        AnalysisResult {
            schema_version: crate::analysis::SCHEMA_VERSION,
            analyzer: crate::analysis::Analyzer::current(),
            coverage: Default::default(),
            project_type: String::new(),
            total_loc: files.iter().map(|f| f.1).sum(),
            comment_lines: 0,
            blank_lines: 0,
            architecture_details: Default::default(),
            dead_code_details: Vec::new(),
            circular_imports: Vec::new(),
            workspaces: Vec::new(),
            languages: HashMap::new(),
            frameworks: vec![],
            databases: vec![],
            infrastructure: vec![],
            architecture: String::new(),
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
    fn counts_changes_inside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit(&repo, &[("a.rs", "1"), ("b.rs", "1")], 500);
        commit(&repo, &[("a.rs", "2")], 400);
        commit(&repo, &[("a.rs", "3")], 10);
        commit(&repo, &[("a.rs", "4")], 5);
        commit(&repo, &[("b.rs", "2")], 1);

        let churn = churn(dir.path(), 365).unwrap();
        assert_eq!(churn.commits, 3);
        assert_eq!(churn.counts.get(Path::new("a.rs")), Some(&2));
        assert_eq!(churn.counts.get(Path::new("b.rs")), Some(&1));
        assert!(!churn.truncated);
    }

    #[test]
    fn paths_are_relative_to_a_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit(&repo, &[("sub/c.rs", "1"), ("top.rs", "1")], 3);
        commit(&repo, &[("sub/c.rs", "2"), ("top.rs", "2")], 2);

        let churn = churn(&dir.path().join("sub"), 365).unwrap();
        assert_eq!(churn.counts.get(Path::new("c.rs")), Some(&1));
        assert_eq!(churn.counts.len(), 1);
    }

    #[test]
    fn ranks_by_commits_times_loc() {
        let mut churn = Churn::default();
        churn.counts.insert(PathBuf::from("small.rs"), 10);
        churn.counts.insert(PathBuf::from("big.rs"), 2);
        let result = result(&[("big.rs", 500), ("small.rs", 20), ("quiet.rs", 900)]);

        let ranked = rank(&result, &churn, 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].file, "./big.rs");
        assert_eq!(ranked[0].score, 1000);
        assert_eq!(ranked[1].file, "./small.rs");
        assert_eq!(rank(&result, &churn, 1).len(), 1);
    }

    #[test]
    fn matches_analysis_paths_with_leading_dot() {
        let mut churn = Churn::default();
        churn.counts.insert(PathBuf::from("src/main.rs"), 3);
        let result = result(&[("./src/main.rs", 10)]);

        let ranked = rank(&result, &churn, 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].file, "./src/main.rs");
        assert_eq!(ranked[0].score, 30);
    }

    #[test]
    fn counts_files_changed_together() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        commit(&repo, &[("a.rs", "0"), ("b.rs", "0"), ("c.rs", "0")], 6);
        for (i, day) in [5, 4, 3].into_iter().enumerate() {
            let body = format!("v{i}");
            commit(
                &repo,
                &[("a.rs", body.as_str()), ("b.rs", body.as_str())],
                day,
            );
        }
        commit(&repo, &[("a.rs", "x"), ("c.rs", "x")], 1);

        let churn = churn(dir.path(), 365).unwrap();
        let ab = (PathBuf::from("a.rs"), PathBuf::from("b.rs"));
        let ac = (PathBuf::from("a.rs"), PathBuf::from("c.rs"));
        assert_eq!(churn.pairs.get(&ab), Some(&3));
        assert_eq!(churn.pairs.get(&ac), Some(&1));

        let result = result(&[("./a.rs", 10), ("./b.rs", 10), ("./c.rs", 10)]);
        let coupled = couple(&result, &churn, 10);
        assert_eq!(
            coupled,
            [Coupling {
                file: "./a.rs".to_string(),
                other: "./b.rs".to_string(),
                shared: 3,
                percent: 100,
            }]
        );
    }

    #[test]
    fn coupling_needs_shared_history_and_source_files() {
        let mut churn = Churn::default();
        churn.counts.insert(PathBuf::from("a.rs"), 10);
        churn.counts.insert(PathBuf::from("b.rs"), 10);
        churn.counts.insert(PathBuf::from("c.rs"), 4);
        churn.counts.insert(PathBuf::from("README.md"), 5);
        churn
            .pairs
            .insert((PathBuf::from("a.rs"), PathBuf::from("b.rs")), 3);
        churn
            .pairs
            .insert((PathBuf::from("a.rs"), PathBuf::from("c.rs")), 4);
        churn
            .pairs
            .insert((PathBuf::from("README.md"), PathBuf::from("a.rs")), 5);
        let result = result(&[("a.rs", 1), ("b.rs", 1), ("c.rs", 1)]);

        let coupled = couple(&result, &churn, 10);
        assert_eq!(coupled.len(), 1);
        assert_eq!(
            (coupled[0].other.as_str(), coupled[0].percent),
            ("./c.rs", 100)
        );
    }

    #[test]
    fn folder_without_history_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let empty = Repository::init(dir.path().join("empty")).unwrap();
        assert!(churn(empty.workdir().unwrap(), 365)
            .unwrap_err()
            .contains("no commits"));
    }

    #[test]
    fn text_lists_hotspots() {
        let report = HotspotReport {
            days: 30,
            commits_scanned: 4,
            truncated: false,
            hotspots: vec![Hotspot {
                file: "src/main.rs".to_string(),
                commits: 4,
                loc: 120,
                score: 480,
            }],
            coupling: vec![Coupling {
                file: "src/main.rs".to_string(),
                other: "src/cli.rs".to_string(),
                shared: 4,
                percent: 80,
            }],
        };
        let text = render_text(&report);
        assert!(text.contains("last 30 days, 4 commits"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("80%  src/main.rs + src/cli.rs"));
    }
}
