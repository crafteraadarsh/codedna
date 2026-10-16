//! Task-focused context.
//!
//! Given a short description of a coding task ("add rate limiting to the
//! login endpoint"), ranks the repository's files by how likely they are to
//! matter for it: files whose names or folders match the task's words, files
//! whose code mentions them, and the files those import or are imported by.
//! Test files are listed separately.
//!
//! Matching is lexical, so it works offline and on any language; the reasons
//! shown with each file say why it was picked.

use crate::analysis::AnalysisResult;
use crate::architecture::is_test;
use crate::safe_text::sanitize;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Files listed for a task.
pub const FILE_LIMIT: usize = 10;

/// Tests listed for a task.
pub const TEST_LIMIT: usize = 5;

/// Longest task description used, in characters.
pub const MAX_TASK_CHARS: usize = 500;

/// Files larger than this are matched on their path only.
const MAX_READ_BYTES: u64 = 512 * 1024;

/// Share of a file's score passed to the files it imports or is imported by.
const NEIGHBOUR_SHARE: f64 = 0.35;

/// A file found only as a neighbour must reach this share of the top score.
const NEIGHBOUR_FLOOR: f64 = 0.15;

/// Prose, data and unrecognised files are matched on their path only: they
/// say a lot without being code to change.
const PATH_ONLY: &[&str] = &["Markdown", "JSON", "Text", "Other"];

/// Reasons shown per file.
const REASON_LIMIT: usize = 4;

#[rustfmt::skip]
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "into", "onto", "that", "this", "these", "those",
    "when", "where", "which", "what", "why", "how", "who", "will", "would", "should", "could",
    "can", "not", "but", "are", "was", "were", "been", "being", "has", "have", "had", "its",
    "our", "your", "their", "them", "they", "you", "all", "any", "each", "some", "more", "most",
    "only", "also", "than", "then", "there", "here", "about", "after", "before", "over", "under",
    "via", "per", "out", "use", "using", "make", "makes", "need", "needs", "want", "add", "adds",
    "added", "adding", "fix", "fixes", "fixed", "fixing", "change", "changes", "update", "updates",
    "implement", "implementing", "support", "supports", "new", "instead", "get", "set", "code",
    "file", "files", "bug", "issue", "feature", "way", "like", "just", "please", "does", "doing",
    "done", "don",
];

/// One file and why it was picked.
#[derive(Debug, Clone, Serialize)]
pub struct TaskFile {
    pub file: String,
    /// Relevance from 1 to 100, relative to the best match for this task.
    pub score: u32,
    /// E.g. `name matches "login"`, `mentions "rate" 4×`, `imported by src/app.ts`.
    pub reasons: Vec<String>,
}

/// Files that matter for a task.
#[derive(Debug, Clone, Serialize)]
pub struct TaskContext {
    pub task: String,
    /// The task's words that were matched, without common words.
    pub terms: Vec<String>,
    pub files: Vec<TaskFile>,
    pub tests: Vec<TaskFile>,
}

/// Rank the files of an analysed repository at `root` for `task`.
pub fn find(root: &Path, result: &AnalysisResult, task: &str) -> TaskContext {
    rank(result, task, |file| {
        let path = root.join(file);
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > MAX_READ_BYTES {
            return None;
        }
        std::fs::read_to_string(&path).ok()
    })
}

/// Rank with file contents supplied by `read`, so tests need no files.
fn rank<F>(result: &AnalysisResult, task: &str, read: F) -> TaskContext
where
    F: Fn(&Path) -> Option<String> + Sync,
{
    let task: String = task.chars().take(MAX_TASK_CHARS).collect();
    let found = terms(&task);
    let label: HashMap<&str, &str> = found
        .iter()
        .map(|(stem, word)| (stem.as_str(), word.as_str()))
        .collect();
    let mut context = TaskContext {
        task: sanitize(task.trim()),
        terms: found.iter().map(|(_, word)| sanitize(word)).collect(),
        files: Vec::new(),
        tests: Vec::new(),
    };
    if found.is_empty() {
        return context;
    }

    // Count each term in every file's code, in parallel.
    let counts: Vec<(&PathBuf, HashMap<String, usize>)> = result
        .file_breakdown
        .par_iter()
        .map(|f| {
            let mut counts = HashMap::new();
            if PATH_ONLY.contains(&f.language.as_str()) {
                return (&f.file, counts);
            }
            if let Some(text) = read(&f.file) {
                let ext = f
                    .file
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let code = crate::code_text::code_only(&ext, &text);
                for word in words(&code) {
                    if label.contains_key(word.as_str()) {
                        *counts.entry(word).or_insert(0) += 1;
                    }
                }
            }
            (&f.file, counts)
        })
        .collect();

    // Rarer terms say more: weight each by inverse document frequency.
    let total = counts.len().max(1) as f64;
    let mut seen_in: HashMap<&str, usize> = HashMap::new();
    for (_, c) in &counts {
        for term in c.keys() {
            *seen_in.entry(term.as_str()).or_insert(0) += 1;
        }
    }
    let idf = |term: &str| {
        let df = seen_in.get(term).copied().unwrap_or(0) as f64;
        ((total + 1.0) / (df + 1.0)).ln() + 0.5
    };

    // Words of each file's name and folders. A word most paths share, like
    // `src`, says nothing about the task.
    let paths: Vec<(HashSet<String>, HashSet<String>)> = counts
        .iter()
        .map(|(file, _)| {
            let name = file
                .file_stem()
                .map(|s| words(&s.to_string_lossy()).collect())
                .unwrap_or_default();
            let dirs = file
                .parent()
                .map(|p| words(&p.to_string_lossy()).collect())
                .unwrap_or_default();
            (name, dirs)
        })
        .collect();
    let common: HashSet<&str> = label
        .keys()
        .copied()
        .filter(|term| {
            let n = paths
                .iter()
                .filter(|(name, dirs)| name.contains(*term) || dirs.contains(*term))
                .count();
            n > 5 && n * 3 > paths.len()
        })
        .collect();

    let mut scores: HashMap<&PathBuf, (f64, Vec<String>)> = HashMap::new();
    for ((file, mentions), (name_words, dir_words)) in counts.iter().zip(&paths) {
        let mut score = 0.0;
        let mut reasons = Vec::new();
        for (term, word) in found.iter().filter(|(t, _)| !common.contains(t.as_str())) {
            if name_words.contains(term) {
                score += 6.0 * idf(term).max(1.0);
                reasons.push(format!("name matches \"{word}\""));
            } else if dir_words.contains(term) {
                score += 3.0 * idf(term).max(1.0);
                reasons.push(format!("folder matches \"{word}\""));
            }
        }
        let mut mentioned: Vec<(&String, &usize)> = mentions.iter().collect();
        mentioned.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (term, n) in mentioned {
            score += idf(term) * (1.0 + (*n as f64).ln()).min(4.0);
            reasons.push(format!("mentions \"{}\" {n}×", label[term.as_str()]));
        }
        if score > 0.0 {
            scores.insert(file, (score, reasons));
        }
    }

    // Files next to strong matches matter too: what they import and what
    // imports them. Only the strongest few pass weight on.
    let mut importers: HashMap<&PathBuf, BTreeSet<&PathBuf>> = HashMap::new();
    for (file, deps) in &result.dependency_graph {
        for dep in deps {
            if dep != file {
                importers.entry(dep).or_default().insert(file);
            }
        }
    }
    let mut seeds: Vec<(&PathBuf, f64)> = scores.iter().map(|(f, (s, _))| (*f, *s)).collect();
    seeds.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    seeds.truncate(FILE_LIMIT);
    let mut passed: HashMap<&PathBuf, (f64, String)> = HashMap::new();
    for (seed, score) in &seeds {
        let imports = result.dependency_graph.get(*seed).into_iter().flatten();
        let imported_by = importers.get(seed).into_iter().flatten().copied();
        let neighbours = imports
            .map(|n| (n, format!("imported by {}", show(seed))))
            .chain(imported_by.map(|n| (n, format!("imports {}", show(seed)))));
        // A file that imports everything (`main.rs`, an index) passes on
        // little to each.
        let degree = result.dependency_graph.get(*seed).map_or(0, Vec::len)
            + importers.get(seed).map_or(0, BTreeSet::len);
        let share = score * NEIGHBOUR_SHARE * (5.0 / degree.max(1) as f64).min(1.0);
        for (neighbour, reason) in neighbours {
            if neighbour == *seed {
                continue;
            }
            let entry = passed.entry(neighbour).or_insert((0.0, String::new()));
            if share > entry.0 {
                *entry = (share, reason);
            }
        }
    }
    let direct: HashSet<&PathBuf> = scores.keys().copied().collect();
    for (file, (share, reason)) in passed {
        let entry = scores.entry(file).or_insert((0.0, Vec::new()));
        entry.0 += share;
        entry.1.push(reason);
    }

    let mut ranked: Vec<(&PathBuf, f64, Vec<String>)> =
        scores.into_iter().map(|(f, (s, r))| (f, s, r)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let top = ranked.first().map(|r| r.1).unwrap_or(1.0).max(f64::EPSILON);
    for (file, score, mut reasons) in ranked {
        // A file found only through a weak link is noise.
        if !direct.contains(file) && score < top * NEIGHBOUR_FLOOR {
            continue;
        }
        reasons.truncate(REASON_LIMIT);
        let entry = TaskFile {
            file: show(file),
            score: ((score / top) * 100.0).round().max(1.0) as u32,
            reasons,
        };
        if is_test(file) {
            if context.tests.len() < TEST_LIMIT {
                context.tests.push(entry);
            }
        } else if context.files.len() < FILE_LIMIT {
            context.files.push(entry);
        }
    }
    context
}

/// The task's meaningful words as `(stem, word)`, in order, without repeats.
fn terms(task: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    task.split(|c: char| !c.is_alphanumeric())
        .flat_map(split_camel)
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .map(|w| (stem(&w), w))
        .filter(|(stem, _)| seen.insert(stem.clone()))
        .collect()
}

/// Lower-case word stems in `text`, split on punctuation, `snake_case`,
/// `kebab-case` and `camelCase`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .flat_map(split_camel)
        .map(|w| stem(&w.to_lowercase()))
}

fn split_camel(word: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    let mut start = 0;
    for i in 1..chars.len() {
        let (at, c) = chars[i];
        let prev = chars[i - 1].1;
        let next_lower = chars.get(i + 1).is_some_and(|(_, n)| n.is_lowercase());
        // `fooBar` → foo|Bar; `HTTPServer` → HTTP|Server.
        if c.is_uppercase() && (prev.is_lowercase() || (prev.is_uppercase() && next_lower)) {
            parts.push(&word[start..at]);
            start = at;
        }
    }
    if start < word.len() {
        parts.push(&word[start..]);
    }
    parts
}

/// Strip common English endings so "routes", "routing" and "route" meet.
/// Deliberately crude: only equality between stems matters.
fn stem(word: &str) -> String {
    let n = word.len();
    if !word.is_ascii() || n <= 3 {
        return word.to_string();
    }
    let sibilant =
        |b: &&str| b.ends_with(['s', 'x', 'z']) || b.ends_with("ch") || b.ends_with("sh");
    let base = if let Some(base) = word.strip_suffix("ies").filter(|_| n > 4) {
        format!("{base}y")
    } else if let Some(base) = word.strip_suffix("ing").filter(|_| n >= 6) {
        undouble(base)
    } else if let Some(base) = word.strip_suffix("ed").filter(|_| n >= 5) {
        undouble(base)
    } else if let Some(base) = word.strip_suffix("es").filter(|b| n >= 5 && sibilant(b)) {
        base.to_string()
    } else if let Some(base) = word
        .strip_suffix('s')
        .filter(|b| !b.ends_with(['s', 'u', 'i']))
    {
        base.to_string()
    } else {
        word.to_string()
    };
    match base.strip_suffix('e') {
        Some(b) if b.len() >= 4 => b.to_string(),
        _ => base,
    }
}

/// `logg` → `log`, `stopp` → `stop`.
fn undouble(base: &str) -> String {
    let b = base.as_bytes();
    let n = b.len();
    if n >= 3 && b[n - 1] == b[n - 2] && !b"lsz".contains(&b[n - 1]) {
        base[..n - 1].to_string()
    } else {
        base.to_string()
    }
}

fn show(path: &Path) -> String {
    sanitize(&path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::FileInfo;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn result(files: &[&str], graph: &[(&str, &[&str])]) -> AnalysisResult {
        AnalysisResult {
            schema_version: crate::analysis::SCHEMA_VERSION,
            analyzer: crate::analysis::Analyzer::current(),
            coverage: Default::default(),
            project_type: String::new(),
            total_loc: 0,
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
            file_breakdown: files
                .iter()
                .map(|f| FileInfo {
                    file: p(f),
                    loc: 10,
                    language: "TypeScript".to_string(),
                })
                .collect(),
            dependency_graph: graph
                .iter()
                .map(|(f, deps)| (p(f), deps.iter().map(|d| p(d)).collect()))
                .collect(),
        }
    }

    fn words_of(task: &str) -> Vec<String> {
        terms(task).into_iter().map(|(_, w)| w).collect()
    }

    #[test]
    fn extracts_terms() {
        assert_eq!(
            words_of("Add rate limiting to the login endpoint"),
            vec!["rate", "limiting", "login", "endpoint"]
        );
        assert_eq!(words_of("fix getUserProfile bug"), vec!["user", "profile"]);
        assert_eq!(words_of("routes and routing"), vec!["routes"]);
        assert!(terms("the and of 42").is_empty());
    }

    #[test]
    fn stems_common_endings() {
        assert_eq!(stem("logins"), "login");
        assert_eq!(stem("logging"), stem("log"));
        assert_eq!(stem("caches"), stem("cache"));
        assert_eq!(stem("matches"), stem("match"));
        assert_eq!(stem("routes"), stem("routing"));
        assert_eq!(stem("queries"), "query");
        assert_eq!(stem("rates"), "rate");
        assert_eq!(stem("status"), "status");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("api"), "api");
    }

    #[test]
    fn splits_camel_case() {
        assert_eq!(
            split_camel("getUserProfile"),
            vec!["get", "User", "Profile"]
        );
        assert_eq!(split_camel("HTTPServer"), vec!["HTTP", "Server"]);
        assert_eq!(split_camel("plain"), vec!["plain"]);
    }

    #[test]
    fn ranks_names_contents_neighbours_and_tests() {
        let r = result(
            &[
                "./src/auth/login.ts",
                "./src/services/session.ts",
                "./src/routes/index.ts",
                "./src/util/strings.ts",
                "./tests/login.test.ts",
            ],
            &[
                ("./src/routes/index.ts", &["./src/auth/login.ts"]),
                ("./src/auth/login.ts", &["./src/services/session.ts"]),
                ("./tests/login.test.ts", &["./src/auth/login.ts"]),
            ],
        );
        let contents: HashMap<PathBuf, &str> = [
            (
                p("./src/auth/login.ts"),
                "export function login(user) { checkRate(user) }",
            ),
            (p("./src/services/session.ts"), "export const session = {}"),
            (p("./src/routes/index.ts"), "router.post('/x', handler)"),
            (
                p("./src/util/strings.ts"),
                "// login is mentioned only in a comment",
            ),
            (p("./tests/login.test.ts"), "test('login', () => login())"),
        ]
        .into_iter()
        .collect();
        let ctx = rank(&r, "Add rate limiting to login", |f| {
            contents.get(f).map(|s| s.to_string())
        });
        assert_eq!(ctx.terms, vec!["rate", "limiting", "login"]);
        assert_eq!(ctx.files[0].file, "./src/auth/login.ts");
        assert_eq!(ctx.files[0].score, 100);
        let reasons = &ctx.files[0].reasons;
        assert!(reasons.contains(&"name matches \"login\"".to_string()));
        assert!(reasons.contains(&"mentions \"rate\" 1×".to_string()));
        let names: Vec<&str> = ctx.files.iter().map(|f| f.file.as_str()).collect();
        assert!(names.contains(&"./src/routes/index.ts"));
        assert!(names.contains(&"./src/services/session.ts"));
        // Comments do not count.
        assert!(!names.contains(&"./src/util/strings.ts"));
        assert_eq!(ctx.tests.len(), 1);
        assert_eq!(ctx.tests[0].file, "./tests/login.test.ts");
        let index = ctx
            .files
            .iter()
            .find(|f| f.file.ends_with("index.ts"))
            .unwrap();
        assert_eq!(index.reasons, vec!["imports ./src/auth/login.ts"]);
    }

    #[test]
    fn empty_task_matches_nothing() {
        let r = result(&["./src/a.ts"], &[]);
        let ctx = rank(&r, "  the  ", |_| Some("the".to_string()));
        assert!(ctx.terms.is_empty());
        assert!(ctx.files.is_empty());
    }
}
