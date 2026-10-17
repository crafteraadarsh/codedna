//! Incremental cache: what each file's content showed on an earlier run, so
//! a repeat run only reads the files that changed.
//!
//! Per file the cache keeps line counts, the layer its code shows, its
//! dependency facts and the dead-code names it mentions, keyed by size and
//! modification time. Imports also depend on which other files exist and on
//! `package.json` / `tsconfig.json`, so they are reused only while the file
//! list and every `.json` and `.toml` file are unchanged. Mentions depend on
//! the dead-code candidates, so they are reused only for the same candidates.
//!
//! The cache lives in the user's cache folder, never in the repository, so a
//! repository cannot ship results for its own files. It is off unless the
//! CLI turns it on; `--no-cache` or `CODEDNA_NO_CACHE=1` keep it off and
//! `CODEDNA_CACHE_DIR` moves it.

use crate::architecture::CodeSign;
use crate::dependency_graph::{normalize, ImportTally};
use crate::facts::DependencyFact;
use crate::loc_counter::LineCounts;
use crate::parsers::ParseStatus;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Bumped when the stored layout changes.
const FORMAT: u32 = 6;
/// Largest cache file read.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Files changed this close to the start of a run are not stored: an edit
/// in the same clock tick would leave size and time unchanged.
const RACY: Duration = Duration::from_secs(2);

static ENABLED: AtomicBool = AtomicBool::new(false);
static EXCLUDED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Turn the cache on for this process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Never cache `root`, e.g. a temporary clone of a remote repository.
pub fn exclude(root: &Path) {
    if let Ok(mut excluded) = EXCLUDED.lock() {
        excluded.push(root.to_path_buf());
    }
}

/// Size and modification time in nanoseconds.
type Stamp = (u64, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), u64::try_from(mtime.as_nanos()).ok()?))
}

/// A cached value that may be "nothing found".
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Known<T> {
    v: Option<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Imports {
    facts: Vec<DependencyFact>,
    readable: bool,
    resolved: usize,
    #[serde(default)]
    ambiguous: usize,
    #[serde(default)]
    inferred: usize,
    unresolved: Vec<(usize, String)>,
    external: usize,
    #[serde(default)]
    parse: ParseStatus,
    #[serde(default)]
    parse_errors: Vec<(usize, String)>,
}

/// Dead-code names a file mentions, `(candidate, file is unused, line)`,
/// and its dynamic loaders, `(line, text)`.
pub type Mentions = (
    Vec<(usize, bool, usize)>,
    Vec<(usize, String, Option<String>)>,
);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loc: Option<Known<[usize; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<Known<(usize, CodeSign)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    imports: Option<Imports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mentions: Option<Known<Mentions>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    format: u32,
    codedna: String,
    root: PathBuf,
    tree: u64,
    dead: u64,
    files: HashMap<PathBuf, Entry>,
}

struct Inner {
    path: PathBuf,
    old: Stored,
    stamps: HashMap<PathBuf, Stamp>,
    cutoff: u64,
    tree: u64,
    /// Fingerprint of the dead-code candidates, once known.
    dead: Mutex<Option<u64>>,
    new: Mutex<HashMap<PathBuf, Entry>>,
}

/// Cached per-file results for one analysis run.
pub struct Cache {
    inner: Option<Inner>,
}

impl Cache {
    /// A cache that stores nothing.
    pub fn off() -> Cache {
        Cache { inner: None }
    }

    /// The cache for `root`, whose files are `files`; off unless enabled.
    pub fn open(root: &Path, files: &[PathBuf]) -> Cache {
        if !ENABLED.load(Ordering::Relaxed) || std::env::var_os("CODEDNA_NO_CACHE").is_some() {
            return Cache::off();
        }
        let Ok(canonical) = std::fs::canonicalize(root) else {
            return Cache::off();
        };
        let excluded = EXCLUDED.lock().map(|e| e.clone()).unwrap_or_default();
        if excluded
            .iter()
            .any(|e| std::fs::canonicalize(e).is_ok_and(|e| canonical.starts_with(e)))
        {
            return Cache::off();
        }
        match dir() {
            Some(dir) => Cache::open_in(&dir, root, &canonical, files),
            None => Cache::off(),
        }
    }

    /// The cache for `root` (canonically `canonical`) kept in `dir`.
    fn open_in(dir: &Path, root: &Path, canonical: &Path, files: &[PathBuf]) -> Cache {
        let path = dir.join(format!(
            "{:016x}.json",
            fnv(canonical.to_string_lossy().as_bytes())
        ));
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let cutoff = u64::try_from(start.saturating_sub(RACY).as_nanos()).unwrap_or(u64::MAX);

        let stamps: HashMap<PathBuf, Stamp> = files
            .par_iter()
            .filter_map(|f| Some((normalize(f), stamp(f)?)))
            .collect();
        // The file list and the settings that steer import resolution.
        let mut tree = Fnv::default();
        tree.add(root.as_os_str().as_encoded_bytes());
        for file in files {
            tree.add(file.as_os_str().as_encoded_bytes());
            let settings = file
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("toml"))
                || file.file_name().is_some_and(|n| n == "pnpm-workspace.yaml");
            if settings {
                let (size, mtime) = stamps.get(&normalize(file)).copied().unwrap_or_default();
                tree.add(&size.to_le_bytes());
                tree.add(&mtime.to_le_bytes());
            }
        }

        let old = load(&path)
            .filter(|s| {
                s.format == FORMAT && s.codedna == env!("CARGO_PKG_VERSION") && s.root == canonical
            })
            .unwrap_or_default();
        Cache {
            inner: Some(Inner {
                path,
                old: Stored {
                    root: canonical.to_path_buf(),
                    ..old
                },
                stamps,
                cutoff,
                tree: tree.0,
                dead: Mutex::new(None),
                new: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The cached value for `file` if its content is unchanged, else
    /// `compute()`, remembered for next time.
    fn get<T: Clone>(
        &self,
        file: &Path,
        pick: impl Fn(&Inner, &Entry) -> Option<T>,
        compute: impl FnOnce() -> T,
        put: impl FnOnce(&mut Entry, T),
    ) -> T {
        let Some(inner) = &self.inner else {
            return compute();
        };
        let file = normalize(file);
        let Some(&(size, mtime)) = inner.stamps.get(&file) else {
            return compute();
        };
        if let Some(old) = inner.old.files.get(&file) {
            if (old.size, old.mtime) == (size, mtime) {
                if let Some(value) = pick(inner, old) {
                    return value;
                }
            }
        }
        let value = compute();
        if mtime < inner.cutoff {
            if let Ok(mut new) = inner.new.lock() {
                let entry = new.entry(file).or_insert_with(|| Entry {
                    size,
                    mtime,
                    ..Entry::default()
                });
                put(entry, value.clone());
            }
        }
        value
    }

    /// Line counts of `file`.
    pub fn loc(
        &self,
        file: &Path,
        compute: impl FnOnce() -> Option<LineCounts>,
    ) -> Option<LineCounts> {
        let to = |c: LineCounts| [c.code, c.comment, c.blank];
        let from = |[code, comment, blank]: [usize; 3]| LineCounts {
            code,
            comment,
            blank,
        };
        self.get(
            file,
            |_, e| e.loc.as_ref().map(|k| k.v.map(from)),
            compute,
            |e, v| e.loc = Some(Known { v: v.map(to) }),
        )
    }

    /// The layer `file`'s code shows.
    pub fn layer(
        &self,
        file: &Path,
        compute: impl FnOnce() -> Option<(usize, CodeSign)>,
    ) -> Option<(usize, CodeSign)> {
        self.get(
            file,
            |_, e| e.layer.as_ref().map(|k| k.v.clone()),
            compute,
            |e, v| e.layer = Some(Known { v }),
        )
    }

    /// The dependency facts of `file` and what happened to its imports.
    pub fn imports(
        &self,
        file: &Path,
        compute: impl FnOnce() -> (Vec<DependencyFact>, ImportTally),
    ) -> (Vec<DependencyFact>, ImportTally) {
        self.get(
            file,
            |inner, e| {
                if inner.old.tree != inner.tree {
                    return None;
                }
                e.imports.as_ref().map(|i| {
                    let tally = ImportTally {
                        readable: i.readable,
                        resolved: i.resolved,
                        ambiguous: i.ambiguous,
                        inferred: i.inferred,
                        unresolved: i.unresolved.clone(),
                        external: i.external,
                        parse: i.parse,
                        parse_errors: i.parse_errors.clone(),
                    };
                    (i.facts.clone(), tally)
                })
            },
            compute,
            |e, (facts, tally)| {
                e.imports = Some(Imports {
                    facts,
                    readable: tally.readable,
                    resolved: tally.resolved,
                    ambiguous: tally.ambiguous,
                    inferred: tally.inferred,
                    unresolved: tally.unresolved,
                    external: tally.external,
                    parse: tally.parse,
                    parse_errors: tally.parse_errors,
                })
            },
        )
    }

    /// Set the dead-code candidates that [`Cache::mentions`] answers for.
    pub fn set_dead(&self, dead: &[PathBuf]) {
        if let Some(inner) = &self.inner {
            let mut fp = Fnv::default();
            for file in dead {
                fp.add(file.as_os_str().as_encoded_bytes());
            }
            if let Ok(mut d) = inner.dead.lock() {
                *d = Some(fp.0);
            }
        }
    }

    /// The dead-code names `file` mentions, for the candidates last given
    /// to [`Cache::set_dead`].
    pub fn mentions(
        &self,
        file: &Path,
        compute: impl FnOnce() -> Option<Mentions>,
    ) -> Option<Mentions> {
        let dead = self
            .inner
            .as_ref()
            .and_then(|i| i.dead.lock().ok().and_then(|d| *d));
        if self.inner.is_some() && dead.is_none() {
            return compute();
        }
        self.get(
            file,
            |inner, e| {
                if Some(inner.old.dead) != dead || inner.old.tree != inner.tree {
                    return None;
                }
                e.mentions.as_ref().map(|k| k.v.clone())
            },
            compute,
            |e, v| e.mentions = Some(Known { v }),
        )
    }

    /// Write the cache back, keeping entries still valid for these files.
    pub fn save(self) {
        let Some(inner) = self.inner else {
            return;
        };
        let new = inner.new.into_inner().unwrap_or_default();
        let dead = inner.dead.into_inner().ok().flatten();
        let mut old = inner.old;
        let tree_same = old.tree == inner.tree;
        let dead_same = dead.is_none_or(|d| d == old.dead) && tree_same;
        let removed = old.files.keys().any(|f| !inner.stamps.contains_key(f));
        if new.is_empty() && !removed && dead_same {
            return;
        }
        let mut files: HashMap<PathBuf, Entry> = HashMap::with_capacity(inner.stamps.len());
        for (file, &(size, mtime)) in &inner.stamps {
            let mut entry = match old.files.remove(file) {
                Some(e) if (e.size, e.mtime) == (size, mtime) => e,
                _ => Entry {
                    size,
                    mtime,
                    ..Entry::default()
                },
            };
            if !tree_same {
                entry.imports = None;
            }
            if !dead_same {
                entry.mentions = None;
            }
            if let Some(n) = new.get(file) {
                entry.loc = n.loc.clone().or(entry.loc);
                entry.layer = n.layer.clone().or(entry.layer);
                entry.imports = n.imports.clone().or(entry.imports);
                entry.mentions = n.mentions.clone().or(entry.mentions);
            }
            let empty = entry.loc.is_none()
                && entry.layer.is_none()
                && entry.imports.is_none()
                && entry.mentions.is_none();
            if !empty {
                files.insert(file.clone(), entry);
            }
        }
        let stored = Stored {
            format: FORMAT,
            codedna: env!("CARGO_PKG_VERSION").to_string(),
            root: old.root,
            tree: inner.tree,
            dead: dead.unwrap_or(old.dead),
            files,
        };
        let _ = write(&inner.path, &stored);
    }
}

fn load(path: &Path) -> Option<Stored> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write(path: &Path, stored: &Stored) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    create_private_dir(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(std::io::BufWriter::new(tmp.as_file_mut()), stored)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Create the cache folder readable only by its owner: it holds file paths
/// and snippets of code.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Where cache files go: `CODEDNA_CACHE_DIR`, else the platform's cache
/// folder.
pub fn dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if let Some(dir) = var("CODEDNA_CACHE_DIR") {
        return Some(dir);
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| d.join("codedna").join("cache"));
    }
    if cfg!(target_os = "macos") {
        return var("HOME").map(|h| h.join("Library/Caches/codedna"));
    }
    var("XDG_CACHE_HOME")
        .map(|d| d.join("codedna"))
        .or_else(|| var("HOME").map(|h| h.join(".cache/codedna")))
}

/// FNV-1a, stable across builds and platforms.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    /// Add `bytes`, followed by a separator so `ab`+`c` differs from `a`+`bc`.
    fn add(&mut self, bytes: &[u8]) {
        for &b in bytes.iter().chain(&[0xff]) {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = Fnv::default();
    h.add(bytes);
    h.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dead_code_detector, dependency_graph, loc_counter, scanner};
    use std::fs;

    /// Write `text` to `path` with a modification time a minute ago, so the
    /// cache keeps it.
    fn put(path: &Path, text: &str, minutes_ago: u64) {
        fs::write(path, text).unwrap();
        let when = SystemTime::now() - Duration::from_secs(60 * minutes_ago);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        put(
            &root.join("package.json"),
            r#"{"name":"t","main":"src/index.ts"}"#,
            1,
        );
        put(
            &root.join("src/index.ts"),
            "import { a } from './a';\n// helper\n\nconsole.log(a);\n",
            1,
        );
        put(&root.join("src/a.ts"), "export const a = 1;\n", 1);
        put(&root.join("src/orphan.ts"), "export const o = 2;\n", 1);
        put(&root.join("src/user.ts"), "// mentions orphan\n", 1);
        dir
    }

    /// Everything the cache stands in for, as comparable text.
    fn run(root: &Path, cache: &Cache) -> String {
        let files = scanner::scan_repository(root);
        let mut loc: Vec<_> = files
            .iter()
            .map(|f| (f.clone(), cache.loc(f, || loc_counter::count_line_kinds(f))))
            .collect();
        loc.sort_by(|a, b| a.0.cmp(&b.0));
        let (graph, tallies) = dependency_graph::build_cached(&files, cache);
        let mut graph: Vec<_> = graph.into_iter().collect();
        graph.sort();
        let mut tallies: Vec<_> = tallies.into_iter().collect();
        tallies.sort_by(|a, b| a.0.cmp(&b.0));
        let graph_map = graph.iter().cloned().collect();
        let dead = dead_code_detector::detect_dead_code(&graph_map);
        let assessed = dead_code_detector::assess_with(root, &dead, &files, &graph_map, cache);
        format!("{loc:?}\n{graph:?}\n{tallies:?}\n{assessed:?}")
    }

    fn cached(dir: &Path, root: &Path) -> Cache {
        let files = scanner::scan_repository(root);
        Cache::open_in(dir, root, &fs::canonicalize(root).unwrap(), &files)
    }

    #[test]
    fn cached_results_match_a_fresh_run() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let fresh = run(repo.path(), &Cache::off());
        let first = cached(store.path(), repo.path());
        assert_eq!(run(repo.path(), &first), fresh);
        first.save();
        assert_eq!(fs::read_dir(store.path()).unwrap().count(), 1);
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), fresh);
    }

    #[test]
    fn unchanged_files_are_not_read_again() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        let before = run(repo.path(), &first);
        first.save();
        // Same size and time, but now a comment: only a cache would miss it.
        let a = repo.path().join("src/a.ts");
        let when = fs::metadata(&a).unwrap().modified().unwrap();
        fs::write(&a, "//port const a = 1;\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_modified(when)
            .unwrap();
        assert_ne!(run(repo.path(), &Cache::off()), before);
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), before);
    }

    #[test]
    fn edits_and_new_files_are_read_again() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        run(repo.path(), &first);
        first.save();
        put(
            &repo.path().join("src/index.ts"),
            "import { a } from './a';\nimport { o } from './orphan';\n",
            2,
        );
        put(&repo.path().join("src/new.ts"), "export {};\n", 1);
        let fresh = run(repo.path(), &Cache::off());
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), fresh);
    }

    #[test]
    fn recent_edits_are_not_stored() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("src/a.ts"), "export const a = 1;\n").unwrap();
        let first = cached(store.path(), repo.path());
        run(repo.path(), &first);
        first.save();
        let stored = load(
            &fs::read_dir(store.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(!stored.files.keys().any(|f| f.ends_with("src/a.ts")));
        assert!(stored.files.keys().any(|f| f.ends_with("src/index.ts")));
    }

    #[test]
    fn another_version_or_a_broken_file_is_ignored() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        let before = run(repo.path(), &first);
        first.save();
        let path = fs::read_dir(store.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        put(&repo.path().join("src/a.ts"), "//port const a = 1;\n", 1);
        let after = run(repo.path(), &Cache::off());
        assert_ne!(after, before);

        let text = fs::read_to_string(&path).unwrap();
        let other = text.replace(
            &format!("\"codedna\":\"{}\"", env!("CARGO_PKG_VERSION")),
            "\"codedna\":\"0.0.0\"",
        );
        assert_ne!(other, text);
        fs::write(&path, other).unwrap();
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), after);

        fs::write(&path, "{not json").unwrap();
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), after);
    }

    #[test]
    fn off_unless_enabled() {
        let repo = repo();
        let files = scanner::scan_repository(repo.path());
        assert!(Cache::open(repo.path(), &files).inner.is_none());
    }

    #[test]
    fn fingerprint_separates_parts() {
        let mut a = Fnv::default();
        a.add(b"ab");
        a.add(b"c");
        let mut b = Fnv::default();
        b.add(b"a");
        b.add(b"bc");
        assert_ne!(a.0, b.0);
    }
}
