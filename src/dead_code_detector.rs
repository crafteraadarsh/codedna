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

use crate::analysis::is_lockfile;
use crate::architecture::{is_test, Confidence, Location};
use crate::cache::Cache;
use crate::dependency_graph::{follows_imports, normalize};
use crate::safe_text::sanitize;
use aho_corasick::{AhoCorasick, AhoCorasickKind};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
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
    "main.tsx",
    "main.js",
    "main.jsx",
    "main.mjs",
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
/// 2. Identify entry points by filename, plus tests and declaration files.
/// 3. BFS from all entry points, marking every transitively reachable file.
/// 4. Files in the graph that were never marked are dead code.
///
/// Returns a sorted `Vec<PathBuf>` of unreachable files.
#[cfg(test)]
pub fn detect_dead_code(graph: &HashMap<PathBuf, Vec<PathBuf>>) -> Vec<PathBuf> {
    detect_dead_code_from(graph, &[], &[])
}

/// [`detect_dead_code`], also starting from `extra_entries`: files a
/// framework adapter names, or directories whose files all load (see
/// [`crate::frameworks`]). In `next_roots` the Next.js adapter decides
/// which files are routes, so the name heuristics for Next files and
/// `pages/` folders do not apply there.
pub fn detect_dead_code_from(
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    extra_entries: &[PathBuf],
    next_roots: &[PathBuf],
) -> Vec<PathBuf> {
    if graph.is_empty() {
        return Vec::new();
    }

    let entries: HashSet<PathBuf> = extra_entries.iter().map(|e| normalize(e)).collect();
    let next_roots: Vec<PathBuf> = next_roots.iter().map(|r| normalize(r)).collect();
    let all_files: HashSet<&PathBuf> = graph.keys().collect();
    let entry_points: Vec<&PathBuf> = all_files
        .iter()
        .copied()
        .filter(|f| {
            let file = normalize(f);
            let in_next = next_roots.iter().any(|root| file.starts_with(root));
            is_entry_point(f)
                || is_loaded_by_tooling(f, in_next)
                || file.ancestors().any(|a| entries.contains(a))
        })
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

/// A manual mock in `__mocks__/` that a test swaps in by path. One next to
/// a `package.json` mocks a dependency instead, so it is only used while the
/// package still depends on it: the test-runner adapter names those.
fn is_module_mock(path: &Path) -> bool {
    path.ancestors()
        .find(|dir| dir.file_name().is_some_and(|n| n == "__mocks__"))
        .and_then(Path::parent)
        .is_some_and(|parent| !parent.join("package.json").is_file())
}

/// File stems that frameworks without an adapter load by convention. The
/// Next.js files (`page`, `layout`, `route`, `middleware`, ...) are named by
/// the Next.js adapter, only in packages that depend on `next`.
const CONVENTION_STEMS: &[&str] = &[
    // React Router / Remix.
    "entry.server",
    "entry.client",
    "root",
    // SvelteKit hooks.
    "hooks",
    "hooks.server",
    "hooks.client",
];

/// Files that task runners and generators (Plop, Gulp, Grunt, Jake) load
/// from the working directory.
const TASK_RUNNER_STEMS: &[&str] = &[
    "plopfile",
    "gulpfile",
    "Gulpfile",
    "gruntfile",
    "Gruntfile",
    "jakefile",
    "Jakefile",
];

/// Whether a test runner, compiler or framework loads the file, so nothing
/// imports it: tests, declaration files, Cargo targets, tool configs and
/// file-based routes. `in_next_root`: the file is in a package where the
/// Next.js adapter decides what `pages/` holds.
fn is_loaded_by_tooling(path: &Path, in_next_root: bool) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.contains(".test.")
        || name.contains(".spec.")
        || name.contains(".stories.")
        || name.contains(".config.")
        || name.ends_with(".d.ts")
        // Type tests that `tsd` and `vitest --typecheck` run.
        || name.contains(".test-d.")
        || name == "conftest.py"
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name.ends_with("_test.py")
        || name == "tests.py"
        || name == "conf.py"
    {
        return true;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let dirs: Vec<&str> = path
        .parent()
        .map(|p| p.iter().filter_map(|c| c.to_str()).collect())
        .unwrap_or_default();

    // Migration and seed runners (Alembic, Django, Knex, TypeORM, Sequelize)
    // load every file in these directories.
    if dirs.iter().any(|d| matches!(*d, "migrations" | "seeders")) {
        return true;
    }

    if ext == "rs" {
        // Cargo compiles every file in these directories as its own crate.
        let target_dir = dirs.last().copied().unwrap_or("");
        let in_target_dir = matches!(
            target_dir,
            "tests" | "benches" | "examples" | "fuzz_targets"
        );
        let in_bin = dirs.ends_with(&["src", "bin"]);
        return in_target_dir || in_bin;
    }

    // Components are used from templates, routers and auto-imports, which
    // the import graph does not follow.
    if matches!(ext, "vue" | "svelte" | "astro") {
        return true;
    }

    if matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts"
    ) {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        // `pages/` routes for Astro, Gatsby, Docusaurus and Nuxt; in a
        // Next.js package the adapter names them.
        let file_routes = (!in_next_root && dirs.contains(&"pages"))
            || dirs.windows(2).any(|w| w == ["app", "routes"]);
        // SvelteKit routes (`+page.ts`, `+server.js`), dotfile configs
        // (`.eslintrc.js`), Stimulus controllers, Storybook setup and
        // Docusaurus theme overrides.
        let by_name = name.starts_with('+')
            || TASK_RUNNER_STEMS.contains(&stem)
            || name.starts_with('.')
            || stem.ends_with("_controller")
            || dirs.contains(&".storybook")
            // Manual mocks that Jest and Vitest swap in by path.
            || is_module_mock(path)
            || dirs.windows(2).any(|w| w == ["src", "theme"]);
        return CONVENTION_STEMS.contains(&stem) || file_routes || by_name;
    }

    false
}

// ---------------------------------------------------------------------------
// Confidence
// ---------------------------------------------------------------------------

/// A dead-code candidate with how sure CodeDna is and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeadCodeCandidate {
    pub file: PathBuf,
    pub confidence: Confidence,
    pub reason: String,
    /// Lines that back the reason: where the name is mentioned, or the
    /// dynamic loader that may load the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Location>,
}

/// Candidates with confidence, falling back to the plain list for results
/// saved before confidence was recorded.
pub fn candidates_of(dead: &[PathBuf], details: &[DeadCodeCandidate]) -> Vec<DeadCodeCandidate> {
    if !details.is_empty() || dead.is_empty() {
        return details.to_vec();
    }
    dead.iter()
        .map(|file| DeadCodeCandidate {
            file: file.clone(),
            confidence: Confidence::Medium,
            reason: "Nothing imports it; no confidence was recorded".to_string(),
            evidence: Vec::new(),
        })
        .collect()
}

/// "2 high, 1 medium, 3 low" — the levels present, most certain first.
pub fn confidence_summary(candidates: &[DeadCodeCandidate]) -> String {
    [Confidence::High, Confidence::Medium, Confidence::Low]
        .iter()
        .filter_map(|level| {
            let n = candidates.iter().filter(|c| c.confidence == *level).count();
            (n > 0).then(|| format!("{n} {}", level.as_str()))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A convention folder, or a starter or example project (`starter-example`,
/// `with-redux-example`) kept for people to copy rather than to run.
fn is_convention_dir(dir: &str) -> bool {
    CONVENTION_DIRS.contains(&dir)
        || dir.starts_with("starter")
        || ["-example", "_example", "-examples", "_examples"]
            .iter()
            .any(|end| dir.ends_with(end))
}

/// Folders that frameworks and tools load by convention, without imports.
const CONVENTION_DIRS: &[&str] = &[
    "migrations",
    "migration",
    "alembic",
    ".github",
    ".husky",
    "seeds",
    "seeders",
    "fixtures",
    "plugins",
    "commands",
    "management",
    "scripts",
    "bin",
    "cron",
    "tasks",
    "jobs",
    "listeners",
    "subscribers",
    "locales",
    "i18n",
    "public",
    "static",
    "templatetags",
    "vendor",
    "emails",
    "examples",
    "example",
    "demo",
];

/// Modules Django (and similar frameworks) import from every installed app
/// package without an explicit import.
const APP_AUTOLOADED: &[&str] = &[
    "models",
    "admin",
    "apps",
    "signals",
    "receivers",
    "tasks",
    "checks",
    "translation",
];

/// File stems too common for a name search to say anything.
const GENERIC_STEMS: &[&str] = &[
    "index",
    "mod",
    "main",
    "init",
    "__init__",
    "utils",
    "util",
    "helpers",
    "types",
    "constants",
    "config",
    "common",
    "base",
    "core",
    "app",
    "server",
    "client",
    "api",
    "test",
    "setup",
];

/// Folder names too common to tie a dynamic loader to a candidate.
const GENERIC_DIRS: &[&str] = &[".", "src", "lib", "app", "source", "pkg", "internal"];

/// Files larger than this are not searched for names or loaders.
const MAX_SEARCH_BYTES: u64 = 1024 * 1024;

/// Longest loader line kept in a reason.
const MAX_LOADER_TEXT: usize = 60;

/// Most mention lines kept as evidence for one candidate.
const MAX_MENTIONS: usize = 3;

/// What one file contributes: `(candidate index, file is unused, line)` for
/// the first mention of each name, plus its dynamic loaders.
type FileFindings = (PathBuf, Vec<(usize, bool, usize)>, Vec<Loader>);

/// A line that loads modules by a computed name, or builds a file path
/// from a template string.
struct Loader {
    file: PathBuf,
    line: usize,
    text: String,
    /// `text` lower-cased once, for matching against every candidate.
    lower: String,
    /// For a built path such as `` `with-${db}.ts` ``: the lines around it,
    /// lower-cased. It points at matching files only, so it never lowers
    /// confidence across the whole repository.
    built: Option<String>,
}

/// Rate each unreachable file:
///
/// - **low**: it sits in a folder loaded by convention (`migrations/`,
///   `plugins/`, …), or a dynamic loader mentions its folder or name.
/// - **medium**: its name appears in another reachable file, or the repo
///   loads modules dynamically, or its name is too generic to check.
/// - **high**: nothing imports it and its name appears nowhere else.
///
/// `files` are all scanned files; their contents are searched for loaders
/// and for each candidate's name.
#[cfg(test)]
pub fn assess(root: &Path, dead: &[PathBuf], files: &[PathBuf]) -> Vec<DeadCodeCandidate> {
    assess_with(root, dead, files, &HashMap::new(), &Cache::off())
}

/// [`assess`], reusing the mentions `cache` holds for files that have not
/// changed. `graph` maps each file to the files it imports.
pub fn assess_with(
    root: &Path,
    dead: &[PathBuf],
    files: &[PathBuf],
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    cache: &Cache,
) -> Vec<DeadCodeCandidate> {
    cache.set_dead(dead);
    if dead.is_empty() {
        return Vec::new();
    }

    // Graph paths are normalised (`src/a.ts`), scanned files may not be
    // (`./src/a.ts`), so compare normalised forms.
    let dead_keys: Vec<PathBuf> = dead.iter().map(|f| normalize(f)).collect();
    let dead_set: HashSet<&PathBuf> = dead_keys.iter().collect();
    let stems: Vec<String> = dead.iter().map(|f| stem_of(f)).collect();
    let searchable: Vec<(usize, &str)> = stems
        .iter()
        .enumerate()
        .filter(|(_, s)| s.len() >= 3 && !GENERIC_STEMS.contains(&s.as_str()))
        .map(|(i, s)| (i, s.as_str()))
        .collect();
    // Names made only of identifier characters are found by splitting each
    // file into words and looking them up, which is much faster than a
    // substring search. The rest (`jest.preset`, `my-module`) still need one.
    let mut by_word: HashMap<String, Vec<usize>, BuildFnv> = HashMap::default();
    let mut other: Vec<(usize, String)> = Vec::new();
    for &(index, stem) in &searchable {
        if stem.bytes().all(is_ident_byte) {
            by_word
                .entry(stem.to_ascii_lowercase())
                .or_default()
                .push(index);
        } else {
            other.push((index, stem.to_string()));
        }
    }
    // Paths, numbered after the names: `setup-tests.ts` or `mocks/gallery`
    // in a config, script or template that loads the file by path.
    let n = dead.len();
    // File names are numbered from `n`, folder-plus-stem forms from `2n`.
    for (index, file) in dead.iter().enumerate() {
        let (name, dir_form) = path_patterns(file);
        // A Python module named in a dotted string: `"theme.FlaskyStyle"`.
        if name.is_some() && file.extension().is_some_and(|e| e == "py") {
            let stem = &stems[index];
            other.push((n + index, format!("\"{stem}.")));
            other.push((n + index, format!("'{stem}.")));
        }
        other.extend(name.map(|p| (n + index, p)));
        other.extend(dir_form.map(|p| (2 * n + index, p)));
    }
    let mut words = WordSet::default();
    for word in by_word.keys() {
        words.add(word.as_bytes());
    }
    let matcher = if other.is_empty() {
        None
    } else {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .kind(Some(AhoCorasickKind::DFA))
            .build(other.iter().map(|(_, s)| s.as_str()))
            .ok()
    };

    // Per candidate: files that mention its name, split by whether the
    // mentioning file is itself unreachable.
    let per_file: Vec<FileFindings> = files
        .par_iter()
        .filter(|file| !is_lockfile(file))
        .filter(|file| fs::metadata(file).is_ok_and(|m| m.len() <= MAX_SEARCH_BYTES))
        .filter_map(|file| {
            let (mentions, loaders) = cache.mentions(file, || {
                let bytes = fs::read(file).ok()?;
                let text = std::str::from_utf8(&bytes).ok()?;
                let file_key = normalize(file);
                let from_dead = dead_set.contains(&file_key);
                let is_doc = is_doc(file);
                let is_source = file
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| follows_imports(&e.to_ascii_lowercase()));
                let mut seen = vec![false; 3 * n];
                let mut mentions: Vec<(usize, bool, usize)> = Vec::new();
                let mut lines = LineIndex::default();
                let mut note = |index: usize, offset: usize| {
                    // `seen` first: comparing paths is slow and names repeat often.
                    // Paths count from configs, scripts and templates; a
                    // source file's `lib/data` is usually an import of
                    // another file of the same name.
                    let path_from_source = is_source && index >= 2 * n;
                    if !(is_doc && index >= n)
                        && !path_from_source
                        && !std::mem::replace(&mut seen[index], true)
                        && dead_keys[index % n] != file_key
                    {
                        mentions.push((index, from_dead, lines.line_of(text, offset)));
                    }
                };
                if !by_word.is_empty() {
                    let mut word = String::new();
                    for token in words.candidates(text.as_bytes()) {
                        word.clear();
                        word.extend(token.iter().map(|b| b.to_ascii_lowercase() as char));
                        let offset = token.as_ptr() as usize - text.as_ptr() as usize;
                        for &index in by_word.get(&word).into_iter().flatten() {
                            note(index, offset);
                        }
                    }
                }
                if let Some(m) = &matcher {
                    for hit in m.find_overlapping_iter(text) {
                        // A dotted string ends in `.`, so its next character
                        // starts the attribute.
                        let end = if other[hit.pattern().as_usize()].1.ends_with('.') {
                            hit.start()
                        } else {
                            hit.end()
                        };
                        if is_word(text, hit.start(), end) {
                            note(other[hit.pattern().as_usize()].0, hit.start());
                        }
                    }
                }
                let loaders = find_loaders(file, text)
                    .into_iter()
                    .map(|l| (l.line, l.text, l.built))
                    .collect();
                Some((mentions, loaders))
            })?;
            let loaders = loaders
                .into_iter()
                .map(|(line, text, built)| Loader {
                    file: file.clone(),
                    line,
                    lower: text.to_ascii_lowercase(),
                    text,
                    built,
                })
                .collect();
            Some((file.clone(), mentions, loaders))
        })
        .collect();

    // Counts, plus the first few mentioning lines in file order.
    // Name mentions first, then path mentions.
    let mut live_mentions = vec![Mentions::default(); 3 * n];
    let mut dead_mentions = vec![Mentions::default(); 3 * n];
    let mut loaders: Vec<Loader> = Vec::new();
    for (file, mentions, file_loaders) in per_file {
        for (index, from_dead, line) in mentions {
            let found = if from_dead {
                &mut dead_mentions[index]
            } else {
                &mut live_mentions[index]
            };
            found.count += 1;
            if found.at.len() < MAX_MENTIONS {
                found.at.push((file.clone(), line));
            }
        }
        loaders.extend(file_loaders);
    }
    loaders.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let live_packages: HashSet<PathBuf> = files
        .iter()
        .map(|f| normalize(f))
        .filter(|f| f.file_name().is_some_and(|n| n == "__init__.py") && !dead_set.contains(f))
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .collect();

    let all_files: HashSet<PathBuf> = files.iter().map(|f| normalize(f)).collect();
    let mut candidates: Vec<DeadCodeCandidate> = dead
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let relative = file.strip_prefix(root).unwrap_or(file);
            let key = normalize(file);
            let in_live_package = key.parent().is_some_and(|p| live_packages.contains(p));
            let found = Found {
                live: &live_mentions[index],
                dead: &dead_mentions[index],
                path_live: [&live_mentions[n + index], &live_mentions[2 * n + index]],
                path_dead: [&dead_mentions[n + index], &dead_mentions[2 * n + index]],
                rule_file: has_rule_file(&key, &all_files),
            };
            let (confidence, reason, at) =
                rate(relative, in_live_package, &stems[index], &found, &loaders);
            DeadCodeCandidate {
                file: file.clone(),
                confidence,
                reason,
                evidence: at
                    .iter()
                    .map(|(path, line)| Location::read(root, path, *line))
                    .collect(),
            }
        })
        .collect();
    // Unreachable files that import a candidate or name its path.
    let mut users: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for key in &dead_keys {
        if let Some(targets) = graph.get(key) {
            users
                .entry(key.clone())
                .or_default()
                .extend(targets.iter().cloned());
        }
    }
    for (index, key) in dead_keys.iter().enumerate() {
        for mentions in [&dead_mentions[n + index], &dead_mentions[2 * n + index]] {
            for (path, _) in &mentions.at {
                users.entry(normalize(path)).or_default().push(key.clone());
            }
        }
    }
    follow_users(&mut candidates, &users);
    // Most certain first, then by path.
    candidates.sort_by(|a, b| b.confidence.cmp(&a.confidence).then(a.file.cmp(&b.file)));
    candidates
}

/// Rates a candidate no more certain than the unreachable files that import
/// it or name its path: if a framework or tool loads such a file, it uses
/// this one too. `users` maps each file to the files it uses.
fn follow_users(candidates: &mut [DeadCodeCandidate], users: &HashMap<PathBuf, Vec<PathBuf>>) {
    let index: HashMap<PathBuf, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (normalize(&c.file), i))
        .collect();
    let mut importers: Vec<Vec<usize>> = vec![Vec::new(); candidates.len()];
    for (from, targets) in users {
        let Some(&from) = index.get(&normalize(from)) else {
            continue;
        };
        for target in targets {
            if let Some(&to) = index.get(&normalize(target)) {
                if to != from {
                    importers[to].push(from);
                }
            }
        }
    }
    // Levels only fall, so this ends.
    let mut changed = true;
    while changed {
        changed = false;
        for to in 0..candidates.len() {
            let Some(&from) = importers[to]
                .iter()
                .min_by_key(|&&i| (candidates[i].confidence, &candidates[i].file))
            else {
                continue;
            };
            let level = candidates[from].confidence;
            if level < candidates[to].confidence {
                let name = file_name(&candidates[from].file);
                candidates[to].confidence = level;
                candidates[to].reason = format!(
                    "Used only by {name}, which is unreachable but may be loaded without an import"
                );
                candidates[to].evidence = Vec::new();
                changed = true;
            }
        }
    }
}

/// How many files mention a name, and where the first few do.
#[derive(Debug, Clone, Default)]
struct Mentions {
    count: usize,
    at: Vec<(PathBuf, usize)>,
}

/// Confidence, reason and the lines that back it.
type Rating = (Confidence, String, Vec<(PathBuf, usize)>);

/// What the search found about one candidate.
struct Found<'a> {
    /// Its name, in reachable and in unreachable files.
    live: &'a Mentions,
    dead: &'a Mentions,
    /// Its path, in reachable or non-source files and in unreachable files.
    /// By file name, then by folder plus stem.
    path_live: [&'a Mentions; 2],
    path_dead: [&'a Mentions; 2],
    /// A `<stem>.yaml` beside it, as rule-test targets have.
    rule_file: bool,
}

/// The path forms a config, script or template uses to load `file`: its
/// file name (`setup-tests.ts`) and its folder plus stem (`mocks/gallery`),
/// skipping forms too common to tie to one file.
fn path_patterns(file: &Path) -> (Option<String>, Option<String>) {
    let stem = stem_of(file);
    let generic = stem.len() < 3 || GENERIC_STEMS.contains(&stem.as_str());
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|name| !generic && *name != stem)
        .map(str::to_string);
    let parent = file
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let dir_form = (!parent.is_empty() && !(generic && GENERIC_DIRS.contains(&parent)))
        .then(|| format!("{parent}/{stem}"));
    (name, dir_form)
}

/// Documentation, whose mentions of a path describe it rather than load it.
fn is_doc(file: &Path) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "mdx" | "markdown" | "rst" | "txt" | "adoc" | "po" | "pot" | "xlf" | "xliff"
    )
}

/// Whether a `<stem>.yaml` or `<stem>.yml` sits beside `file`: rule test
/// targets (Semgrep and similar) that the rule's runner loads.
fn has_rule_file(file: &Path, files: &HashSet<PathBuf>) -> bool {
    // Autofix tests pair `rule.fixed.py` with `rule.yaml` too.
    let unfixed = file.with_extension("");
    let unfixed = (unfixed.extension() == Some("fixed".as_ref())).then_some(unfixed);
    ["yaml", "yml"].iter().any(|ext| {
        files.contains(&file.with_extension(ext))
            || unfixed
                .as_ref()
                .is_some_and(|f| files.contains(&f.with_extension(ext)))
    })
}

fn rate(
    file: &Path,
    in_live_package: bool,
    stem: &str,
    found: &Found,
    loaders: &[Loader],
) -> Rating {
    let live_mentions = found.live;
    let dead_mentions = found.dead;
    let dirs: Vec<String> = file
        .parent()
        .map(|p| {
            p.iter()
                .filter_map(|c| c.to_str())
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_default();

    if let Some(dir) = dirs.iter().find(|d| is_convention_dir(d)) {
        return (
            Confidence::Low,
            format!("In {dir}/, a folder tools and frameworks often load by convention"),
            Vec::new(),
        );
    }

    if dirs.windows(2).any(|w| w[0] == "app" && w[1] == "views") {
        return (
            Confidence::Low,
            "In app/views/, which Rails renders by convention".to_string(),
            Vec::new(),
        );
    }

    if is_test(file) || stem == "conftest" {
        return (
            Confidence::Low,
            "A test file, run by the test runner rather than imported".to_string(),
            Vec::new(),
        );
    }

    let is_python = file.extension().is_some_and(|e| e == "py");
    if is_python && in_live_package && APP_AUTOLOADED.contains(&stem) {
        return (
            Confidence::Low,
            format!("Frameworks such as Django load {stem}.py from every installed app"),
            Vec::new(),
        );
    }

    if let Some((path, line)) = found.path_live.iter().find_map(|m| m.at.first()) {
        return (
            Confidence::Low,
            format!(
                "Its path is referenced in {} (a config, script or template may load it)",
                file_name(path)
            ),
            vec![(path.clone(), *line)],
        );
    }

    if found.rule_file {
        return (
            Confidence::Low,
            format!("A test target for the rule in {stem}.yaml beside it"),
            Vec::new(),
        );
    }

    let parent = dirs.last().map(String::as_str).unwrap_or(".");
    let generic_stem = stem.len() < 3 || GENERIC_STEMS.contains(&stem);
    if let Some(l) = loaders
        .iter()
        .find(|l| l.built.as_deref().is_some_and(|w| builds(w, file, stem)))
    {
        return (
            Confidence::Low,
            format!(
                "May be read by a path built at runtime: \"{}\" ({}:{})",
                l.text,
                file_name(&l.file),
                l.line
            ),
            vec![(l.file.clone(), l.line)],
        );
    }
    let loaders: Vec<&Loader> = loaders.iter().filter(|l| l.built.is_none()).collect();
    let related = loaders.iter().find(|l| {
        (!GENERIC_DIRS.contains(&parent) && l.lower.contains(parent))
            || (!generic_stem && l.lower.contains(stem))
    });
    if let Some(l) = related {
        return (
            Confidence::Low,
            format!(
                "May be loaded dynamically: \"{}\" ({}:{})",
                l.text,
                file_name(&l.file),
                l.line
            ),
            vec![(l.file.clone(), l.line)],
        );
    }

    if generic_stem {
        return (
            Confidence::Medium,
            format!("Nothing imports it; the name \"{stem}\" is too common to search for"),
            Vec::new(),
        );
    }
    if live_mentions.count > 0 {
        let n = live_mentions.count;
        return (
            Confidence::Medium,
            format!(
                "Nothing imports it, but \"{stem}\" is mentioned in {n} other file{}",
                if n == 1 { "" } else { "s" }
            ),
            live_mentions.at.clone(),
        );
    }
    if let Some(l) = loaders.first() {
        return (
            Confidence::Medium,
            format!(
                "Nothing imports it, but the repository loads modules dynamically ({}:{})",
                file_name(&l.file),
                l.line
            ),
            vec![(l.file.clone(), l.line)],
        );
    }
    if let Some((path, line)) = found.path_dead.iter().find_map(|m| m.at.first()) {
        return (
            Confidence::Medium,
            format!(
                "Nothing reachable imports it, but its path appears in {}, itself unused",
                file_name(path)
            ),
            vec![(path.clone(), *line)],
        );
    }
    if dead_mentions.count > 0 {
        return (
            Confidence::High,
            format!("Nothing imports it; \"{stem}\" appears only in other unused files"),
            dead_mentions.at.clone(),
        );
    }
    (
        Confidence::High,
        format!("Nothing imports it and \"{stem}\" appears in no other file"),
        Vec::new(),
    )
}

/// Line numbers for byte offsets, built on first use.
#[derive(Default)]
struct LineIndex {
    starts: Option<Vec<usize>>,
}

impl LineIndex {
    /// 1-based line holding `offset`.
    fn line_of(&mut self, text: &str, offset: usize) -> usize {
        let starts = self.starts.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(
                    text.bytes()
                        .enumerate()
                        .filter(|(_, b)| *b == b'\n')
                        .map(|(i, _)| i + 1),
                )
                .collect()
        });
        starts.partition_point(|&start| start <= offset)
    }
}

/// Lower-cased file stem without secondary extensions (`user.service.ts` →
/// `user.service`).
fn stem_of(file: &Path) -> String {
    file.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn file_name(path: &Path) -> String {
    sanitize(
        &path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default(),
    )
}

/// Whether `text[start..end]` is not part of a longer identifier.
fn is_ident_byte(b: u8) -> bool {
    IDENT[b as usize]
}

const IDENT: [bool; 256] = {
    let mut table = [false; 256];
    let mut b = 0;
    while b < 256 {
        let c = b as u8;
        table[b] = c.is_ascii_alphanumeric() || c == b'_';
        b += 1;
    }
    table
};

/// A cheap first check for whole-word lookups: the lengths, the first two
/// letters and the last letter (lowercased) of the words searched for. Most
/// words in a file fail it, so few need to be lowercased and looked up.
struct WordSet {
    /// Bit `n` is set when some word is `n` bytes long (longer ones share bit 63).
    lengths: u64,
    starts: Vec<bool>,
    last: [bool; 256],
}

impl Default for WordSet {
    fn default() -> Self {
        WordSet {
            lengths: 0,
            starts: vec![false; 1 << 16],
            last: [false; 256],
        }
    }
}

impl WordSet {
    fn add(&mut self, word: &[u8]) {
        if word.len() < 2 {
            return;
        }
        self.lengths |= 1 << word.len().min(63);
        self.starts[start_key(word)] = true;
        self.last[word[word.len() - 1].to_ascii_lowercase() as usize] = true;
    }

    /// The whole identifier words in `text` that pass the check.
    fn candidates<'a>(&'a self, text: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        let mut i = 0;
        std::iter::from_fn(move || {
            while i < text.len() {
                if !IDENT[text[i] as usize] {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < text.len() && IDENT[text[i] as usize] {
                    i += 1;
                }
                let word = &text[start..i];
                if word.len() >= 2
                    && self.lengths & (1 << word.len().min(63)) != 0
                    && self.starts[start_key(word)]
                    && self.last[word[word.len() - 1].to_ascii_lowercase() as usize]
                {
                    return Some(word);
                }
            }
            None
        })
    }
}

/// FNV-1a: the words looked up are short, and SipHash dominates the search.
#[derive(Default)]
struct Fnv(u64);

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for &b in bytes {
            hash = (hash ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = hash;
    }
}

type BuildFnv = std::hash::BuildHasherDefault<Fnv>;

fn start_key(word: &[u8]) -> usize {
    ((word[0].to_ascii_lowercase() as usize) << 8) | word[1].to_ascii_lowercase() as usize
}

fn is_word(text: &str, start: usize, end: usize) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let before = text[..start].chars().next_back().is_none_or(|c| !ident(c));
    let after = text[end..].chars().next().is_none_or(|c| !ident(c));
    before && after
}

/// Lines that load modules by a computed name: `import(name)`,
/// `require(path)`, `require.context`, `import.meta.glob`,
/// `importlib.import_module` and `__import__`.
fn find_loaders(file: &Path, text: &str) -> Vec<Loader> {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let js = matches!(
        ext,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" | "astro"
    );
    let py = ext == "py";
    if !js && !py {
        return Vec::new();
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') {
            continue;
        }
        let dynamic = if js {
            trimmed.contains("require.context(")
                || trimmed.contains("import.meta.glob")
                || computed_call(trimmed, "import(")
                || computed_call(trimmed, "require(")
        } else {
            trimmed.contains("import_module(") || trimmed.contains("__import__(")
        };
        // A template string ending in a source extension builds a path to
        // a file that is copied or read rather than imported.
        let built = (js && !dynamic && trimmed.contains("${"))
            .then(|| {
                let ahead = lines[i..lines.len().min(i + 4)].join("\n");
                let builds_source = BUILT_EXTENSIONS
                    .iter()
                    .any(|ext| ahead.contains(&format!(".{ext}`")));
                builds_source.then(|| {
                    lines[i.saturating_sub(3)..lines.len().min(i + 4)]
                        .join("\n")
                        .to_ascii_lowercase()
                })
            })
            .flatten();
        if dynamic || built.is_some() {
            let mut text: String = sanitize(trimmed).chars().take(MAX_LOADER_TEXT).collect();
            if trimmed.chars().count() > MAX_LOADER_TEXT {
                text.push('…');
            }
            let text = text.replace('`', "'");
            found.push(Loader {
                file: file.to_path_buf(),
                line: i + 1,
                lower: text.to_ascii_lowercase(),
                text,
                built,
            });
        }
    }
    found
}

/// Extensions a built path must end in to count: `` `./x/${name}.ts` ``.
const BUILT_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "vue", "svelte", "py",
];

/// Whether a path built from a template string in `window` can name
/// `file`: a literal folder and name prefix just before `${`
/// (`` `index-drizzle/with-${db}.ts` ``), or its folder quoted beside it
/// (`` join(root, "db/schema-drizzle", `${a}-${b}.ts`) ``).
fn builds(window: &str, file: &Path, stem: &str) -> bool {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() || !window.contains(&format!(".{ext}`")) {
        return false;
    }
    let dirs: Vec<String> = file
        .parent()
        .map(|p| {
            p.iter()
                .filter_map(|c| c.to_str())
                .filter(|c| *c != ".")
                .map(str::to_ascii_lowercase)
                .collect()
        })
        .unwrap_or_default();
    let stem = stem.to_ascii_lowercase();
    for (at, _) in window.match_indices("${") {
        let start = window[..at]
            .rfind(|c: char| matches!(c, '`' | '"' | '\'' | '}') || c.is_whitespace())
            .map_or(0, |i| i + 1);
        let fragment = &window[start..at];
        let (folder, name) = fragment.rsplit_once('/').unwrap_or(("", fragment));
        if !stem.starts_with(name) {
            continue;
        }
        let folders: Vec<&str> = folder
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        if folders.is_empty() {
            if name.len() >= 4 {
                return true;
            }
            continue;
        }
        let tail = dirs.len().checked_sub(folders.len()).map(|s| &dirs[s..]);
        if tail.is_some_and(|t| t.iter().zip(&folders).all(|(a, b)| a == b))
            && (name.len() >= 3 || folders.iter().any(|d| !GENERIC_DIRS.contains(d)))
        {
            return true;
        }
    }
    dirs.last().is_some_and(|parent| {
        !GENERIC_DIRS.contains(&parent.as_str())
            && ['"', '\''].iter().any(|q| {
                window.contains(&format!("/{parent}{q}"))
                    || window.contains(&format!("{q}{parent}{q}"))
            })
    })
}

/// Whether `call` appears with an argument that is not a plain string
/// literal: `import(name)` or `` import(`./x/${name}`) ``.
fn computed_call(line: &str, call: &str) -> bool {
    line.match_indices(call).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
            return false;
        }
        let arg = line[at + call.len()..].trim_start();
        match arg.chars().next() {
            Some('"' | '\'') | Some(')') | None => false,
            Some('`') => arg.contains("${"),
            Some(_) => true,
        }
    })
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

    /// Whether tooling loads the file outside any Next.js package.
    fn tooling(path: &Path) -> bool {
        is_loaded_by_tooling(path, false)
    }

    /// Helper: build a graph from (file, deps) pairs.
    fn make_graph(edges: &[(&str, &[&str])]) -> HashMap<PathBuf, Vec<PathBuf>> {
        edges
            .iter()
            .map(|(file, deps)| (p(file), deps.iter().map(|d| p(d)).collect()))
            .collect()
    }

    // -----------------------------------------------------------------------
    // assess
    // -----------------------------------------------------------------------

    fn temp_repo(prefix: &str, files: &[(&str, &str)]) -> (PathBuf, Vec<PathBuf>) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        let mut paths = Vec::new();
        for (name, content) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, content).expect("write");
            paths.push(path);
        }
        (root, paths)
    }

    fn rated<'a>(c: &'a [DeadCodeCandidate], root: &Path, name: &str) -> &'a DeadCodeCandidate {
        c.iter()
            .find(|c| c.file == root.join(name))
            .unwrap_or_else(|| panic!("{name} not rated"))
    }

    #[test]
    fn assess_rates_by_evidence() {
        let (root, files) = temp_repo(
            "codedna_dead_assess",
            &[
                ("src/index.ts", "import { a } from './live';\n"),
                ("src/live.ts", "// see legacyParser for the old way\n"),
                ("src/orphan.ts", "export const x = 1;\n"),
                ("src/legacyParser.ts", "export const y = 2;\n"),
                ("src/dusty.ts", "import './orphan';\n"),
                ("src/utils.ts", "export const z = 3;\n"),
                ("db/migrations/001_init.ts", "export const up = 1;\n"),
                ("src/live.test.ts", "test('x', () => {});\n"),
            ],
        );
        let dead: Vec<PathBuf> = files
            .iter()
            .filter(|f| !f.ends_with("index.ts") && !f.ends_with("live.ts"))
            .cloned()
            .collect();
        let c = assess(&root, &dead, &files);

        let legacy = rated(&c, &root, "src/legacyParser.ts");
        assert_eq!(legacy.confidence, Confidence::Medium, "{}", legacy.reason);
        assert!(legacy.reason.contains("mentioned in 1 other file"));
        assert_eq!(legacy.evidence.len(), 1);
        assert_eq!(legacy.evidence[0].file, "src/live.ts");
        assert_eq!(legacy.evidence[0].line, 1);
        assert!(legacy.evidence[0].text.contains("legacyParser"));

        let orphan = rated(&c, &root, "src/orphan.ts");
        assert_eq!(orphan.confidence, Confidence::High, "{}", orphan.reason);
        assert!(orphan.reason.contains("only in other unused files"));

        assert_eq!(
            rated(&c, &root, "src/dusty.ts").confidence,
            Confidence::High
        );
        assert_eq!(
            rated(&c, &root, "src/utils.ts").confidence,
            Confidence::Medium
        );
        assert_eq!(
            rated(&c, &root, "db/migrations/001_init.ts").confidence,
            Confidence::Low
        );
        assert_eq!(
            rated(&c, &root, "src/live.test.ts").confidence,
            Confidence::Low
        );

        // Most certain first.
        assert!(c.windows(2).all(|w| w[0].confidence >= w[1].confidence));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn line_index_counts_from_one() {
        let text = "a\nbb\n\nccc";
        let mut lines = LineIndex::default();
        assert_eq!(lines.line_of(text, 0), 1);
        assert_eq!(lines.line_of(text, 1), 1);
        assert_eq!(lines.line_of(text, 2), 2);
        assert_eq!(lines.line_of(text, 5), 3);
        assert_eq!(lines.line_of(text, 6), 4);
    }

    #[test]
    fn dynamic_loaders_lower_confidence() {
        let (root, files) = temp_repo(
            "codedna_dead_dynamic",
            &[
                ("src/index.js", "const h = require(`./handlers/${name}`);\n"),
                ("src/handlers/refund.js", "module.exports = 1;\n"),
                ("src/other/stray.js", "module.exports = 2;\n"),
            ],
        );
        let dead = vec![files[1].clone(), files[2].clone()];
        let c = assess(&root, &dead, &files);

        let refund = rated(&c, &root, "src/handlers/refund.js");
        assert_eq!(refund.confidence, Confidence::Low, "{}", refund.reason);
        assert!(refund.reason.contains("index.js:1"));
        assert_eq!(refund.evidence[0].file, "src/index.js");
        assert_eq!(refund.evidence[0].line, 1);

        let stray = rated(&c, &root, "src/other/stray.js");
        assert_eq!(stray.confidence, Confidence::Medium, "{}", stray.reason);
        assert!(stray.reason.contains("dynamically"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn files_named_by_a_built_path_are_low() {
        let (root, files) = temp_repo(
            "codedna_dead_built_path",
            &[
                (
                    "cli/install.ts",
                    "const a = join(dir, `db/index-drizzle/with-${provider}.ts`);\n\
                     const b = join(\n  dir,\n  \"db/schema-drizzle\",\n  `${base}-${provider}.ts`\n);\n",
                ),
                ("template/db/index-drizzle/with-mysql.ts", "export const db = 1;\n"),
                ("template/db/schema-drizzle/base-postgres.ts", "export const t = 1;\n"),
                ("src/unrelated/stray-widget.ts", "export const s = 1;\n"),
            ],
        );
        let dead = vec![files[1].clone(), files[2].clone(), files[3].clone()];
        let c = assess(&root, &dead, &files);

        for path in [
            "template/db/index-drizzle/with-mysql.ts",
            "template/db/schema-drizzle/base-postgres.ts",
        ] {
            let built = rated(&c, &root, path);
            assert_eq!(built.confidence, Confidence::Low, "{}", built.reason);
            assert!(built.reason.contains("install.ts"), "{}", built.reason);
        }
        // A built path says nothing about other files.
        let stray = rated(&c, &root, "src/unrelated/stray-widget.ts");
        assert_eq!(stray.confidence, Confidence::High, "{}", stray.reason);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn django_app_modules_are_low() {
        let (root, files) = temp_repo(
            "codedna_dead_django",
            &[
                ("shop/__init__.py", ""),
                ("shop/admin.py", "admin.site.register(X)\n"),
                ("shop/forms_unused.py", "class F: pass\n"),
            ],
        );
        let dead = vec![files[1].clone(), files[2].clone()];
        let c = assess(&root, &dead, &files);
        assert_eq!(
            rated(&c, &root, "shop/admin.py").confidence,
            Confidence::Low
        );
        assert_eq!(
            rated(&c, &root, "shop/forms_unused.py").confidence,
            Confidence::High
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn path_references_and_conventions_lower_confidence() {
        let (root, files) = temp_repo(
            "codedna_dead_paths",
            &[
                ("vitest.config.ts", "setupFiles: ['./src/setup-tests.ts']\n"),
                ("src/setup-tests.ts", "export {}\n"),
                ("src/old-runner.ts", "run('./src/old_job.ts')\n"),
                ("src/old_job.ts", "export {}\n"),
                ("rules/no-eval.ts", "eval(x)\n"),
                ("rules/no-eval.yaml", "rules: []\n"),
                ("rules/no-eval.fixed.py", "pass\n"),
                (
                    "docs/conf.py",
                    "pygments_style = \"theme_support.Flasky\"\n",
                ),
                ("docs/theme_support.py", "class Flasky: pass\n"),
                (
                    "app/views/pwa/service-worker.js",
                    "self.addEventListener('x')\n",
                ),
                ("dashboard/starter-example/app/ui/search.tsx", "export {}\n"),
            ],
        );
        let live = ["vitest.config.ts", "rules/no-eval.yaml", "docs/conf.py"];
        let dead: Vec<PathBuf> = files
            .iter()
            .filter(|f| !live.iter().any(|l| f.ends_with(l)))
            .cloned()
            .collect();
        let c = assess(&root, &dead, &files);
        let level = |name| rated(&c, &root, name).confidence;

        assert_eq!(level("src/setup-tests.ts"), Confidence::Low);
        // Named only by a file that is itself unused.
        assert_eq!(level("src/old_job.ts"), Confidence::Medium);
        assert_eq!(level("rules/no-eval.ts"), Confidence::Low);
        assert_eq!(level("rules/no-eval.fixed.py"), Confidence::Low);
        assert_eq!(level("docs/theme_support.py"), Confidence::Low);
        assert_eq!(level("app/views/pwa/service-worker.js"), Confidence::Low);
        assert_eq!(
            level("dashboard/starter-example/app/ui/search.tsx"),
            Confidence::Low
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn files_used_by_uncertain_files_are_no_more_certain() {
        let (root, files) = temp_repo(
            "codedna_dead_users",
            &[
                ("emails/welcome.tsx", "import { Card } from '../ui/Card';\n"),
                ("ui/Card.tsx", "export const Card = 1;\n"),
                ("ui/Orphaned.tsx", "export const O = 1;\n"),
            ],
        );
        let mut graph = make_graph(&[]);
        graph.insert(normalize(&files[0]), vec![normalize(&files[1])]);
        let c = assess_with(&root, &files, &files, &graph, &Cache::off());

        let card = rated(&c, &root, "ui/Card.tsx");
        assert_eq!(card.confidence, Confidence::Low);
        assert!(card.reason.contains("welcome.tsx"));
        assert_eq!(
            rated(&c, &root, "ui/Orphaned.tsx").confidence,
            Confidence::High
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn self_mentions_count_whatever_the_path_prefix() {
        let (root, files) = temp_repo(
            "codedna_dead_prefix",
            &[("src/legacy_thing.ts", "// legacy_thing: old code\n")],
        );
        // Scanned as `./src/...`, unreachable as `src/...`.
        let scanned: Vec<PathBuf> = files
            .iter()
            .map(|f| root.join(".").join(f.strip_prefix(&root).unwrap()))
            .collect();
        let c = assess(&root, &files, &scanned);

        assert_eq!(c[0].confidence, Confidence::High);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn computed_call_ignores_literals() {
        assert!(computed_call("const m = await import(name);", "import("));
        assert!(computed_call("require(`./x/${a}`)", "require("));
        assert!(!computed_call("const m = await import('./x');", "import("));
        assert!(!computed_call("require(\"fs\")", "require("));
        assert!(!computed_call("my_require(x)", "require("));
        assert!(!computed_call("require(`./plain`)", "require("));
    }

    #[test]
    fn word_boundaries() {
        assert!(is_word("see legacy here", 4, 10));
        assert!(!is_word("seelegacy here", 3, 9));
        assert!(!is_word("legacy_x", 0, 6));
    }

    #[test]
    fn summary_and_fallback() {
        let dead = vec![p("./a.ts"), p("./b.ts")];
        let c = candidates_of(&dead, &[]);
        assert_eq!(c.len(), 2);
        assert_eq!(confidence_summary(&c), "2 medium");
        assert_eq!(confidence_summary(&[]), "");
    }

    // -----------------------------------------------------------------------
    // is_entry_point
    // -----------------------------------------------------------------------

    #[test]
    fn recognises_cargo_targets() {
        assert!(tooling(&p("tests/cli.rs")));
        assert!(tooling(&p("crates/core/benches/speed.rs")));
        assert!(tooling(&p("examples/basic.rs")));
        assert!(tooling(&p("src/bin/tool.rs")));
        assert!(!tooling(&p("src/scanner.rs")));
        assert!(!tooling(&p("src/bin/helpers/x.rs")));
    }

    #[test]
    fn recognises_framework_conventions() {
        // Next.js files are named by the Next.js adapter, not by name.
        assert!(!tooling(&p("app/dashboard/page.tsx")));
        assert!(!tooling(&p("app/layout.tsx")));
        assert!(!tooling(&p("app/api/users/route.ts")));
        assert!(!tooling(&p("middleware.ts")));
        assert!(!tooling(&p("instrumentation-client.ts")));
        // `pages/` stays a route folder (Astro, Gatsby, Docusaurus) except
        // in a Next.js package, where the adapter decides.
        assert!(tooling(&p("src/pages/about.tsx")));
        assert!(!is_loaded_by_tooling(&p("src/pages/about.tsx"), true));
        assert!(tooling(&p("app/root.tsx")));
        assert!(tooling(&p("src/hooks.server.ts")));
        assert!(tooling(&p("app/routes/_index.tsx")));
        assert!(tooling(&p("postcss.config.mjs")));
        assert!(tooling(&p("src/Button.stories.tsx")));
        assert!(tooling(&p("src/form.test-d.ts")));
        assert!(tooling(&p("src/__mocks__/axios.ts")));
        assert!(tooling(&p("src/components/Button.vue")));
        assert!(tooling(&p("src/lib/Modal.svelte")));
        assert!(tooling(&p("plopfile.js")));
        assert!(tooling(&p(
            "api/src/database/migrations/20240716A-dates.ts"
        )));
        assert!(tooling(&p(
            "backend/migrations/versions/d4c1a8e37b62_add.py"
        )));
        assert!(!tooling(&p("src/form.ts")));
        assert!(!tooling(&p("src/components/Button.tsx")));
        assert!(!tooling(&p("src/page.py")));
    }

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
        assert!(is_entry_point(&p("src/main.tsx")));
        assert!(is_entry_point(&p("src/main.jsx")));
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
    fn recognises_files_loaded_by_tooling() {
        assert!(tooling(&p("src/app.test.ts")));
        assert!(tooling(&p("e2e/server.spec.ts")));
        assert!(tooling(&p("src/types/user-request.d.ts")));
        assert!(tooling(&p("tests/test_users.py")));
        assert!(tooling(&p("tests/users_test.py")));
        assert!(tooling(&p("tests/conftest.py")));
        assert!(!tooling(&p("src/utils.ts")));
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

    /// Dead files of `graph`, starting from what the adapters find in
    /// `files`.
    fn dead_with_adapters(
        graph: &HashMap<PathBuf, Vec<PathBuf>>,
        files: &[PathBuf],
    ) -> Vec<PathBuf> {
        let adapters = crate::frameworks::detect(files);
        detect_dead_code_from(graph, &adapters.entry_files(), &adapters.next_roots)
    }

    fn graph_of(files: &[PathBuf]) -> HashMap<PathBuf, Vec<PathBuf>> {
        files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e != "json"))
            .map(|f| (f.clone(), vec![]))
            .collect()
    }

    #[test]
    fn mocks_of_dependencies_load_only_while_depended_on() {
        let (root, files) = temp_repo(
            "codedna_mocks",
            &[
                (
                    "web/package.json",
                    r#"{"dependencies": {"zustand": "4"}, "devDependencies": {"@scope/ui": "1"}}"#,
                ),
                ("web/__mocks__/zustand.ts", ""),
                ("web/__mocks__/@scope/ui.js", ""),
                ("web/__mocks__/removed-package.js", ""),
                ("web/src/__mocks__/api.ts", ""),
            ],
        );
        assert_eq!(
            dead_with_adapters(&graph_of(&files), &files),
            vec![root.join("web/__mocks__/removed-package.js")]
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn package_json_names_entry_points() {
        let (root, files) = temp_repo(
            "codedna_manifest",
            &[
                (
                    "apps/worker/package.json",
                    r#"{"main": "dist/index.js", "bin": {"w": "./cli.mjs"},
                    "scripts": {"start": "ts-node worker.ts", "seed": "node ./seed.js", "lint": "eslint src"},
                    "prisma": {"seed": "tsx prisma/seed.ts"}}"#,
                ),
                ("apps/worker/cli.mjs", ""),
                ("apps/worker/worker.ts", ""),
                ("apps/worker/seed.js", ""),
                ("apps/worker/prisma/seed.ts", ""),
                ("apps/worker/workers/indexing.ts", ""),
                ("apps/worker/src/unused.ts", ""),
            ],
        );
        let dir = root.join("apps/worker");
        let entries = crate::frameworks::detect(&files).entry_files();
        for name in ["cli.mjs", "worker.ts", "seed.js", "prisma/seed.ts"] {
            assert!(entries.contains(&dir.join(name)), "{name}");
        }
        // `dist/` is not there, and `eslint src` names a folder, not a file.
        assert!(!entries.iter().any(|e| e.starts_with(dir.join("dist"))));
        assert!(!entries.iter().any(|e| e.starts_with(dir.join("src"))));

        let worker = dir.join("worker.ts");
        let job = dir.join("workers/indexing.ts");
        let mut graph = graph_of(&files);
        graph.insert(worker, vec![job]);
        assert_eq!(
            dead_with_adapters(&graph, &files),
            vec![dir.join("src/unused.ts")]
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn expo_router_directories_are_entry_points() {
        let (root, files) = temp_repo(
            "codedna_expo",
            &[
                (
                    "apps/mobile/package.json",
                    r#"{"main": "expo-router/entry", "dependencies": {"expo-router": "~4.0.0"}}"#,
                ),
                (
                    "apps/web/package.json",
                    r#"{"dependencies": {"next": "15"}}"#,
                ),
                ("apps/mobile/app/(tabs)/_layout.tsx", ""),
                ("apps/mobile/components/Sheet.tsx", ""),
                ("apps/mobile/components/Unused.tsx", ""),
                ("apps/web/app/settings.tsx", ""),
            ],
        );
        let layout = root.join("apps/mobile/app/(tabs)/_layout.tsx");
        let sheet = root.join("apps/mobile/components/Sheet.tsx");
        let mut graph = graph_of(&files);
        graph.insert(layout, vec![sheet]);
        assert_eq!(
            dead_with_adapters(&graph, &files),
            vec![
                root.join("apps/mobile/components/Unused.tsx"),
                root.join("apps/web/app/settings.tsx")
            ]
        );

        fs::remove_dir_all(root).ok();
    }
}
