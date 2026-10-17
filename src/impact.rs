//! Change impact: what a change touches beyond the files it edits.
//!
//! The changed files come from Git: everything that differs between a base
//! (by default `HEAD`, so uncommitted work) and the working tree, untracked
//! files included. For a branch base such as `origin/main` the comparison
//! starts at the merge base, as a pull request would. Every file that
//! imports a changed file, directly or through other files, is affected.
//! The report then names the layers, API routes and tests involved.
//!
//! Each affected file says how the change reaches it: through runtime
//! imports, through an import that may load one of several files
//! (possible), or only through type imports (type-only), by the strongest
//! chain of imports that reaches it.
//!
//! Only imports the dependency graph follows are seen, so a file reached
//! some other way (reflection, configuration, a language whose imports are
//! not followed) is not listed. Imports that did not resolve but could name
//! a changed file are listed apart, as places the impact may be incomplete.

use crate::architecture::{is_test, layer_by_file, LAYER_NAMES};
use crate::facts::{DependencyFact, Resolution};
use crate::graph::{EdgeKind, TypedGraph};
use crate::runbook::Route;
use crate::safe_text::sanitize;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Base compared against when none is given.
pub const DEFAULT_BASE: &str = "HEAD";
/// Most files listed in each of affected, routes and tests.
pub const MAX_LISTED: usize = 50;
/// Largest file read for routes.
const MAX_ROUTE_FILE_BYTES: u64 = 512 * 1024;

/// A file the change edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub file: String,
    /// `added`, `modified`, `deleted` or `renamed`.
    pub status: String,
    /// Layer the file sits in, when it is placed in one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

/// A file that imports changed code, directly or through other files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AffectedFile {
    pub file: String,
    /// 1 when it imports a changed file itself, 2 through one other file, …
    pub depth: usize,
    /// The changed or affected file it imports that brings it in.
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// How the change reaches it, by the strongest chain of imports.
    pub kind: Link,
}

/// How a change reaches an affected file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// Every import on the way is a runtime import.
    Runtime,
    /// One import on the way may load one of several files (an ambiguous
    /// import or a lazy pattern), none is type-only.
    Possible,
    /// One import on the way is type-only: the file is checked against the
    /// change, its runtime behaviour is not affected through this chain.
    TypeOnly,
}

impl Link {
    fn note(self) -> &'static str {
        match self {
            Link::Runtime => "",
            Link::Possible => " (possibly)",
            Link::TypeOnly => " (types only)",
        }
    }
}

/// An import that did not resolve and may name a changed file: the change
/// may reach further than listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedImport {
    /// The file with the import.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub specifier: String,
    /// The changed file it may name.
    pub may_name: String,
}

/// Files changed and affected in one layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerImpact {
    pub name: String,
    pub changed: usize,
    pub affected: usize,
}

/// What a change touches.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Impact {
    /// What the working tree was compared with, e.g. `origin/main (1a2b3c4)`.
    pub base: String,
    pub changed: Vec<ChangedFile>,
    /// Files that import changed code, nearest first; tests are listed
    /// separately. At most [`MAX_LISTED`].
    pub affected: Vec<AffectedFile>,
    /// All affected files, before the list was cut.
    pub affected_total: usize,
    /// Layers with changed or affected files, top-down.
    pub layers: Vec<LayerImpact>,
    /// API routes declared in changed or affected files.
    pub routes: Vec<Route>,
    /// Test files that import changed code: the ones to run.
    pub tests: Vec<String>,
    /// Imports that did not resolve and may name a changed file. At most
    /// [`MAX_LISTED`].
    pub unresolved: Vec<UnresolvedImport>,
}

/// Compare the working tree under `root` with `base` and work out what the
/// change affects.
pub fn analyze(root: &Path, base: &str) -> Result<Impact, String> {
    let (base, changed) = changed_files(root, base)?;
    let files = crate::scanner::scan_repository(root);
    let (graph, _, facts) =
        crate::dependency_graph::build_facts(&files, &crate::cache::Cache::off());
    let layers: HashMap<PathBuf, &'static str> = layer_by_file(root, &files, &graph)
        .into_iter()
        .map(|(file, layer)| (repo_path(root, &file), layer))
        .collect();
    let facts: Vec<DependencyFact> = facts
        .into_iter()
        .map(|f| f.map_paths(|p| repo_path(root, p)))
        .collect();
    let graph = crate::graph::typed_edges(graph.keys().map(|f| repo_path(root, f)), &facts);
    let mut impact = trace(&graph, &layers, changed);
    impact.unresolved = unresolved_near(&facts, &impact.changed);
    impact.base = base;
    impact.routes = routes_in(root, &impact);
    Ok(impact)
}

/// Files that differ between `base` and the working tree, as paths relative
/// to `root`, with a description of the base actually compared.
fn changed_files(root: &Path, base: &str) -> Result<(String, Vec<(PathBuf, String)>), String> {
    let no_git = || "No Git repository found here: change impact needs one.".to_string();
    let repo = git2::Repository::discover(root).map_err(|_| no_git())?;
    let workdir = repo
        .workdir()
        .and_then(|w| w.canonicalize().ok())
        .ok_or_else(no_git)?;
    let prefix = root
        .canonicalize()
        .ok()
        .and_then(|r| r.strip_prefix(&workdir).ok().map(Path::to_path_buf))
        .ok_or_else(no_git)?;

    let shown = sanitize(base);
    let base_commit = repo
        .revparse_single(base)
        .and_then(|o| o.peel_to_commit())
        .map_err(|_| {
            format!(
                "Can't find `{shown}` in this repository. Fetch it first (for example \
                 `git fetch origin main`) or pass another --base."
            )
        })?;
    let head = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    // A branch is compared from where the work left it, as a pull request is.
    let from = match &head {
        Some(head) if head.id() != base_commit.id() => repo
            .merge_base(base_commit.id(), head.id())
            .ok()
            .and_then(|oid| repo.find_commit(oid).ok()),
        _ => None,
    };
    let short = |c: &git2::Commit| c.id().to_string()[..7].to_string();
    let (from, described) = match from {
        Some(merge) if merge.id() != base_commit.id() => {
            let described = format!("merge base of {shown} and HEAD ({})", short(&merge));
            (merge, described)
        }
        _ => {
            let described = format!("{shown} ({})", short(&base_commit));
            (base_commit, described)
        }
    };
    let tree = from.tree().map_err(|e| sanitize(&e.to_string()))?;

    let mut options = git2::DiffOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let mut diff = repo
        .diff_tree_to_workdir_with_index(Some(&tree), Some(&mut options))
        .map_err(|e| sanitize(&e.to_string()))?;
    diff.find_similar(None)
        .map_err(|e| sanitize(&e.to_string()))?;

    let mut changed: BTreeMap<PathBuf, String> = BTreeMap::new();
    for delta in diff.deltas() {
        let status = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked | git2::Delta::Copied => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Modified | git2::Delta::Typechange => "modified",
            _ => continue,
        };
        let file = if status == "deleted" {
            delta.old_file().path()
        } else {
            delta.new_file().path()
        };
        if let Some(file) = file.and_then(|f| f.strip_prefix(&prefix).ok()) {
            changed.insert(file.to_path_buf(), status.to_string());
        }
    }
    Ok((described, changed.into_iter().collect()))
}

/// Follow imports backwards from the changed files.
///
/// Each file is reached by its strongest chain: through runtime imports
/// only if it can be, else through possible ones too, else through any
/// import, type-only ones included. Within that, breadth-first, so it gets
/// its shortest such path to the change.
fn trace(
    graph: &TypedGraph,
    layers: &HashMap<PathBuf, &'static str>,
    changed: Vec<(PathBuf, String)>,
) -> Impact {
    let mut importers: HashMap<&PathBuf, BTreeMap<&PathBuf, EdgeKind>> = HashMap::new();
    for (file, deps) in graph {
        for (dep, kind) in deps {
            if dep != file {
                let slot = importers
                    .entry(dep)
                    .or_default()
                    .entry(file)
                    .or_insert(*kind);
                *slot = (*slot).min(*kind);
            }
        }
    }
    let layer = |file: &Path| layers.get(file).map(|l| l.to_string());

    let mut starts: Vec<&PathBuf> = changed.iter().map(|(f, _)| f).collect();
    starts.sort();
    let mut seen: BTreeMap<&PathBuf, (usize, &PathBuf, Link)> = BTreeMap::new();
    let stages = [
        (Link::Runtime, EdgeKind::Runtime),
        (Link::Possible, EdgeKind::Possible),
        (Link::TypeOnly, EdgeKind::Type),
    ];
    for (link, weakest) in stages {
        // Every file reached so far, nearest first, spreads through edges
        // no weaker than this stage allows.
        let mut queue: VecDeque<(&PathBuf, usize)> = starts.iter().map(|f| (*f, 0)).collect();
        let mut reached: Vec<(&PathBuf, usize)> =
            seen.iter().map(|(f, (d, _, _))| (*f, *d)).collect();
        reached.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        queue.extend(reached);
        while let Some((file, depth)) = queue.pop_front() {
            for (importer, kind) in importers.get(file).into_iter().flatten() {
                if *kind > weakest || starts.contains(importer) || seen.contains_key(importer) {
                    continue;
                }
                seen.insert(importer, (depth + 1, file, link));
                queue.push_back((importer, depth + 1));
            }
        }
    }

    let mut affected: Vec<AffectedFile> = Vec::new();
    let mut tests: Vec<String> = changed
        .iter()
        .filter(|(f, status)| status != "deleted" && is_test(f))
        .map(|(f, _)| shown(f))
        .collect();
    for (file, (depth, via, link)) in &seen {
        if is_test(file) {
            tests.push(shown(file));
        } else {
            affected.push(AffectedFile {
                file: shown(file),
                depth: *depth,
                via: shown(via),
                layer: layer(file),
                kind: *link,
            });
        }
    }
    affected.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.file.cmp(&b.file)));
    tests.sort();
    tests.dedup();

    let mut counts: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let order = |name: &str| {
        LAYER_NAMES
            .iter()
            .position(|n| *n == name)
            .unwrap_or(usize::MAX)
    };
    let changed: Vec<ChangedFile> = changed
        .into_iter()
        .map(|(file, status)| ChangedFile {
            layer: layer(&file),
            file: shown(&file),
            status,
        })
        .collect();
    for c in &changed {
        if let Some(l) = &c.layer {
            counts.entry(order(l)).or_default().0 += 1;
        }
    }
    for a in &affected {
        if let Some(l) = &a.layer {
            counts.entry(order(l)).or_default().1 += 1;
        }
    }
    let layers = counts
        .into_iter()
        .filter_map(|(index, (changed, affected))| {
            Some(LayerImpact {
                name: LAYER_NAMES.get(index)?.to_string(),
                changed,
                affected,
            })
        })
        .collect();

    let affected_total = affected.len();
    affected.truncate(MAX_LISTED);
    tests.truncate(MAX_LISTED);
    Impact {
        base: String::new(),
        changed,
        affected,
        affected_total,
        layers,
        routes: Vec::new(),
        tests,
        unresolved: Vec::new(),
    }
}

/// Imports that did not resolve and could name one of the changed files
/// (the same path without its extension, a file inside the folder it
/// names, or an alias ending in the same segments). An unresolved import
/// that could name none of them is left out.
fn unresolved_near(facts: &[DependencyFact], changed: &[ChangedFile]) -> Vec<UnresolvedImport> {
    let changed: Vec<PathBuf> = changed.iter().map(|c| PathBuf::from(&c.file)).collect();
    let mut out: Vec<UnresolvedImport> = Vec::new();
    for fact in facts {
        if fact.resolution != Resolution::Unresolved || changed.contains(&fact.source) {
            continue;
        }
        for file in &changed {
            if crate::dead_code_detector::could_name(&fact.source, &fact.specifier, file) {
                out.push(UnresolvedImport {
                    file: shown(&fact.source),
                    line: fact.evidence.line,
                    specifier: sanitize(&fact.specifier),
                    may_name: shown(file),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then_with(|| a.may_name.cmp(&b.may_name))
    });
    out.dedup();
    out.truncate(MAX_LISTED);
    out
}

/// API routes declared in the changed and affected files.
fn routes_in(root: &Path, impact: &Impact) -> Vec<Route> {
    let files = impact
        .changed
        .iter()
        .filter(|c| c.status != "deleted")
        .map(|c| &c.file)
        .chain(impact.affected.iter().map(|a| &a.file));
    let mut routes: Vec<Route> = files
        .filter_map(|file| {
            let path = root.join(file);
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_ROUTE_FILE_BYTES {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            Some(crate::runbook::routes(
                file,
                &String::from_utf8_lossy(&bytes),
            ))
        })
        .flatten()
        .collect();
    routes.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then_with(|| a.path.cmp(&b.path))
    });
    routes.dedup();
    routes.truncate(MAX_LISTED);
    routes
}

/// `path` relative to `root`, without the leading `./`, as Git names it.
fn repo_path(root: &Path, path: &Path) -> PathBuf {
    let relative = crate::analysis::relative_path(root, path);
    relative
        .strip_prefix(".")
        .map(Path::to_path_buf)
        .unwrap_or(relative)
}

fn shown(path: &Path) -> String {
    sanitize(&path.to_string_lossy().replace('\\', "/"))
}

fn status_mark(status: &str) -> char {
    match status {
        "added" => 'A',
        "deleted" => 'D',
        "renamed" => 'R',
        _ => 'M',
    }
}

fn layer_note(layer: &Option<String>) -> String {
    layer
        .as_ref()
        .map(|l| format!("  [{l}]"))
        .unwrap_or_default()
}

/// Plain-text report for the terminal.
pub fn render_text(impact: &Impact) -> String {
    let mut out = format!("Change impact against {}\n", impact.base);
    if impact.changed.is_empty() {
        out.push_str("\nNo changes.\n");
        return out;
    }
    let _ = writeln!(out, "\nChanged ({})", impact.changed.len());
    for c in &impact.changed {
        let _ = writeln!(
            out,
            "  {} {}{}",
            status_mark(&c.status),
            c.file,
            layer_note(&c.layer)
        );
    }
    if impact.affected_total == 0 {
        out.push_str("\nNo other file imports the changed files.\n");
    } else {
        let _ = writeln!(out, "\nAffects ({} files)", impact.affected_total);
        for a in &impact.affected {
            let _ = writeln!(
                out,
                "  {}{}  imports {}{}",
                a.file,
                layer_note(&a.layer),
                a.via,
                a.kind.note()
            );
        }
        if impact.affected_total > impact.affected.len() {
            let _ = writeln!(
                out,
                "  … and {} more",
                impact.affected_total - impact.affected.len()
            );
        }
    }
    if !impact.unresolved.is_empty() {
        let _ = writeln!(
            out,
            "\nMay reach further: {} import(s) did not resolve and may name a changed file",
            impact.unresolved.len()
        );
        for u in &impact.unresolved {
            let _ = writeln!(
                out,
                "  {}  imports {}  (may be {})",
                at(u),
                u.specifier,
                u.may_name
            );
        }
    }
    if !impact.layers.is_empty() {
        let layers: Vec<String> = impact.layers.iter().map(describe_layer).collect();
        let _ = writeln!(out, "\nLayers: {}", layers.join(", "));
    }
    if !impact.routes.is_empty() {
        let _ = writeln!(out, "\nAPI routes ({})", impact.routes.len());
        for r in &impact.routes {
            let _ = writeln!(out, "  {} {}  {}:{}", r.method, r.path, r.file, r.line);
        }
    }
    if !impact.tests.is_empty() {
        let _ = writeln!(out, "\nTests to run ({})", impact.tests.len());
        for t in &impact.tests {
            let _ = writeln!(out, "  {t}");
        }
    }
    out
}

/// `file:line` of an unresolved import.
fn at(u: &UnresolvedImport) -> String {
    match u.line {
        Some(line) => format!("{}:{line}", u.file),
        None => u.file.clone(),
    }
}

fn describe_layer(l: &LayerImpact) -> String {
    match (l.changed, l.affected) {
        (0, a) => format!("{} ({a} affected)", l.name),
        (c, 0) => format!("{} ({c} changed)", l.name),
        (c, a) => format!("{} ({c} changed, {a} affected)", l.name),
    }
}

/// Markdown report, for a pull request comment or CI job summary.
pub fn render_markdown(impact: &Impact) -> String {
    let code = |s: &str| format!("`{}`", s.replace('`', "'"));
    let mut out = format!("### Change impact\n\nAgainst {}.\n", impact.base);
    if impact.changed.is_empty() {
        out.push_str("\nNo changes.\n");
        return out;
    }
    let _ = writeln!(
        out,
        "\n**{} changed**, **{} affected** through imports",
        impact.changed.len(),
        impact.affected_total
    );
    if !impact.layers.is_empty() {
        let layers: Vec<String> = impact.layers.iter().map(describe_layer).collect();
        let _ = writeln!(out, "\nLayers: {}", layers.join(", "));
    }
    let _ = writeln!(out, "\n| | Changed file | Layer |\n|---|---|---|");
    for c in &impact.changed {
        let _ = writeln!(
            out,
            "| {} | {} | {} |",
            status_mark(&c.status),
            code(&c.file),
            c.layer.as_deref().unwrap_or("")
        );
    }
    if !impact.affected.is_empty() {
        let _ = writeln!(
            out,
            "\n<details><summary>Affected files ({})</summary>\n\n| File | Layer | Imports |\n|---|---|---|",
            impact.affected_total
        );
        for a in &impact.affected {
            let _ = writeln!(
                out,
                "| {} | {} | {}{} |",
                code(&a.file),
                a.layer.as_deref().unwrap_or(""),
                code(&a.via),
                a.kind.note()
            );
        }
        out.push_str("\n</details>\n");
    }
    if !impact.unresolved.is_empty() {
        let _ = writeln!(
            out,
            "\n**May reach further**: these imports did not resolve and may name a changed file\n"
        );
        for u in &impact.unresolved {
            let _ = writeln!(
                out,
                "- {} imports {} (may be {})",
                code(&at(u)),
                code(&u.specifier),
                code(&u.may_name)
            );
        }
    }
    if !impact.routes.is_empty() {
        let _ = writeln!(out, "\n**API routes**\n");
        for r in &impact.routes {
            let _ = writeln!(
                out,
                "- {} {}",
                code(&format!("{} {}", r.method, r.path)),
                code(&format!("{}:{}", r.file, r.line))
            );
        }
    }
    if !impact.tests.is_empty() {
        let _ = writeln!(out, "\n**Tests to run**\n");
        for t in &impact.tests {
            let _ = writeln!(out, "- {}", code(t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn traces_importers_layers_and_tests() {
        let r = |s: &str| (p(s), EdgeKind::Runtime);
        let graph: TypedGraph = [
            (p("src/db/user.ts"), vec![]),
            (p("src/services/user.ts"), vec![r("src/db/user.ts")]),
            (p("src/routes/users.ts"), vec![r("src/services/user.ts")]),
            (p("src/index.ts"), vec![r("src/routes/users.ts")]),
            (p("src/other.ts"), vec![]),
            (p("tests/user.test.ts"), vec![r("src/services/user.ts")]),
        ]
        .into_iter()
        .collect();
        let layers: HashMap<PathBuf, &'static str> = [
            (p("src/db/user.ts"), "Data"),
            (p("src/services/user.ts"), "Services"),
            (p("src/routes/users.ts"), "Routes"),
        ]
        .into_iter()
        .collect();
        let impact = trace(
            &graph,
            &layers,
            vec![(p("src/db/user.ts"), "modified".to_string())],
        );
        let affected: Vec<(&str, usize, &str)> = impact
            .affected
            .iter()
            .map(|a| (a.file.as_str(), a.depth, a.via.as_str()))
            .collect();
        assert_eq!(
            affected,
            [
                ("src/services/user.ts", 1, "src/db/user.ts"),
                ("src/routes/users.ts", 2, "src/services/user.ts"),
                ("src/index.ts", 3, "src/routes/users.ts"),
            ]
        );
        assert_eq!(impact.tests, ["tests/user.test.ts"]);
        let layers: Vec<String> = impact.layers.iter().map(describe_layer).collect();
        assert_eq!(
            layers,
            [
                "Routes (1 affected)",
                "Services (1 affected)",
                "Data (1 changed)"
            ]
        );
        assert_eq!(impact.changed[0].layer.as_deref(), Some("Data"));
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn reads_changes_from_git() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("package.json", "{\"name\":\"x\",\"main\":\"src/index.js\"}");
        write("src/db.js", "module.exports = {};\n");
        write(
            "src/routes.js",
            "const db = require('./db');\napp.get('/users', h);\n",
        );
        write("src/index.js", "require('./routes');\n");
        write("src/gone.js", "module.exports = 1;\n");
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "one"]);

        let unchanged = analyze(root, "HEAD").unwrap();
        assert!(unchanged.changed.is_empty());
        assert!(render_text(&unchanged).contains("No changes."));

        git(root, &["checkout", "-q", "-b", "work"]);
        write("src/db.js", "module.exports = { db: 1 };\n");
        git(root, &["commit", "-q", "-am", "two"]);
        std::fs::remove_file(root.join("src/gone.js")).unwrap();
        write("src/new.js", "require('./db');\n");

        let impact = analyze(root, "main").unwrap();
        let changed: Vec<(&str, &str)> = impact
            .changed
            .iter()
            .map(|c| (c.file.as_str(), c.status.as_str()))
            .collect();
        assert_eq!(
            changed,
            [
                ("src/db.js", "modified"),
                ("src/gone.js", "deleted"),
                ("src/new.js", "added"),
            ]
        );
        let affected: Vec<&str> = impact.affected.iter().map(|a| a.file.as_str()).collect();
        assert_eq!(affected, ["src/routes.js", "src/index.js"]);
        assert_eq!(impact.routes.len(), 1);
        assert_eq!(impact.routes[0].path, "/users");
        assert!(impact.base.starts_with("main ("), "{}", impact.base);
        let md = render_markdown(&impact);
        assert!(md.contains("**3 changed**, **2 affected**"), "{md}");

        let err = analyze(root, "no-such-branch").unwrap_err();
        assert!(err.contains("no-such-branch"), "{err}");
    }
}
