//! Incremental cache: what each file's content showed on an earlier run, so
//! a repeat run only redoes the work whose inputs changed.
//!
//! Per file the cache keeps line counts, the layer its code shows, its
//! JavaScript syntax scan, its dependency facts and the dead-code names it
//! mentions. Every value is keyed by a hash of the file's bytes and by the
//! version of the code that produced it, so an edit that keeps the size and
//! the modification time is still seen, and a new parser or rule set
//! recomputes only what it affects. Hashing every file read is the default;
//! `CODEDNA_CACHE_TRUST_MTIME=1` is the explicit faster mode that trusts an
//! unchanged size and modification time instead of reading the file.
//!
//! Dependency facts also depend on other files: which files exist, the
//! `package.json` and `tsconfig.json` chain, workspace members, framework
//! markers. While a file is resolved, every such lookup is recorded (see
//! [`crate::resolution::deps`]) and stored with the fingerprint of its
//! value; the facts are reused only while every one still matches. Mentions
//! depend on the dead-code candidates, so they are reused only for the same
//! candidates.
//!
//! The cache lives in the user's cache folder, never in the repository, so a
//! repository cannot ship results for its own files. It is off unless the
//! CLI turns it on; `--no-cache` or `CODEDNA_NO_CACHE=1` keep it off and
//! `CODEDNA_CACHE_DIR` moves it. `CODEDNA_CACHE_STATS=1` prints how many
//! values of each kind were reused and computed.

use crate::architecture::CodeSign;
use crate::dependency_graph::{normalize, ImportTally};
use crate::facts::{DependencyFact, RULES_VERSION};
use crate::loc_counter::LineCounts;
use crate::parsers::javascript::{JsScan, PARSER_VERSION};
use crate::parsers::ParseStatus;
use crate::resolution::deps::{self, Dep};
use crate::resolution::javascript::RESOLVER_VERSION;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Bumped when the stored layout changes.
const FORMAT: u32 = 9;
/// Largest cache file read.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Files changed this close to the start of a run are not stored: the file
/// may still be changing while it is read.
const RACY: Duration = Duration::from_secs(2);

/// Bump when line counting gives different counts for the same text.
const LOC_VERSION: u32 = 1;
/// Bump when the layer read from code changes for the same text.
const LAYER_VERSION: u32 = 1;
/// Bump when the dead-code mention search changes.
const MENTIONS_VERSION: u32 = 1;

static ENABLED: AtomicBool = AtomicBool::new(false);
static EXCLUDED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Turn the cache on for this process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Turn the cache off again, e.g. to compare a cached run with an uncached one.
pub fn disable() {
    ENABLED.store(false, Ordering::Relaxed);
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

/// The 128-bit hash of a file's bytes.
type Hash = [u64; 2];

fn hash_file(path: &Path) -> Option<Hash> {
    let bytes = std::fs::read(path).ok()?;
    let h = xxhash_rust::xxh3::xxh3_128(&bytes);
    Some([(h >> 64) as u64, h as u64])
}

/// The version of the code behind each kind of cached value. A stored
/// value made by another version is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct Versions {
    loc: String,
    layer: String,
    scan: String,
    imports: String,
    mentions: String,
}

impl Versions {
    fn current() -> Versions {
        Versions {
            loc: LOC_VERSION.to_string(),
            layer: LAYER_VERSION.to_string(),
            scan: PARSER_VERSION.to_string(),
            imports: format!(
                "rules {RULES_VERSION}; resolver {RESOLVER_VERSION}; {PARSER_VERSION}"
            ),
            mentions: MENTIONS_VERSION.to_string(),
        }
    }
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
    ambiguous: usize,
    inferred: usize,
    unresolved: Vec<(usize, String)>,
    external: usize,
    parse: ParseStatus,
    parse_errors: Vec<(usize, String)>,
    /// What the resolution read, as indices into [`Stored::deps`].
    deps: Vec<u32>,
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
    hash: Hash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loc: Option<Known<[usize; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<Known<Vec<(usize, CodeSign)>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scan: Option<JsScan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    imports: Option<Imports>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mentions: Option<Known<Mentions>>,
}

impl Entry {
    fn is_empty(&self) -> bool {
        self.loc.is_none()
            && self.layer.is_none()
            && self.scan.is_none()
            && self.imports.is_none()
            && self.mentions.is_none()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    format: u32,
    codedna: String,
    root: PathBuf,
    versions: Versions,
    /// Fingerprint of the dead-code candidates the mentions are for.
    dead: u64,
    /// Every lookup a stored resolution made, with the fingerprint of the
    /// value it saw.
    deps: Vec<(Dep, u64)>,
    files: HashMap<PathBuf, Entry>,
}

/// The kinds of cached value, for [`Stats`].
#[derive(Clone, Copy)]
enum Kind {
    Loc,
    Layer,
    Scan,
    Imports,
    Mentions,
}

const KINDS: [(Kind, &str); 5] = [
    (Kind::Loc, "loc"),
    (Kind::Layer, "layer"),
    (Kind::Scan, "scan"),
    (Kind::Imports, "imports"),
    (Kind::Mentions, "mentions"),
];

/// How many values of each kind were reused and computed.
#[derive(Default)]
struct Stats([[AtomicU64; 2]; 5]);

impl Stats {
    fn count(&self, kind: Kind, reused: bool) {
        self.0[kind as usize][usize::from(!reused)].fetch_add(1, Ordering::Relaxed);
    }

    fn json(&self) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        for (kind, name) in KINDS {
            let [reused, computed] = &self.0[kind as usize];
            out.insert(
                name.to_string(),
                serde_json::json!({
                    "reused": reused.load(Ordering::Relaxed),
                    "computed": computed.load(Ordering::Relaxed),
                }),
            );
        }
        serde_json::Value::Object(out)
    }
}

/// A file of this run.
struct Seen {
    stamp: Stamp,
    /// Its content hash, read the first time a value is asked for; `None`
    /// when it cannot be read.
    hash: OnceLock<Option<Hash>>,
}

struct Inner {
    path: PathBuf,
    old: Stored,
    /// Whether the stored versions differ from the current ones.
    versions_changed: bool,
    files: HashMap<PathBuf, Seen>,
    cutoff: u64,
    /// Reuse a stored hash when the size and time are unchanged.
    trust_mtime: bool,
    /// Fingerprint of the dead-code candidates, once known.
    dead: Mutex<Option<u64>>,
    new: Mutex<HashMap<PathBuf, Entry>>,
    /// What each new resolution read, with the fingerprints it saw.
    new_deps: Mutex<HashMap<PathBuf, Vec<(Dep, u64)>>>,
    /// Current fingerprints of the lookups checked this run.
    observed: Mutex<HashMap<Dep, u64>>,
    stats: Stats,
}

impl Inner {
    /// The content hash of `file`.
    fn hash(&self, file: &Path, seen: &Seen) -> Option<Hash> {
        *seen.hash.get_or_init(|| {
            if self.trust_mtime {
                if let Some(old) = self.old.files.get(file) {
                    if (old.size, old.mtime) == seen.stamp {
                        return Some(old.hash);
                    }
                }
            }
            hash_file(file)
        })
    }

    /// The current fingerprint of `dep`, asked of `observe` once per run.
    fn observe(&self, dep: &Dep, observe: &impl Fn(&Dep) -> u64) -> u64 {
        if let Some(&v) = self
            .observed
            .lock()
            .ok()
            .and_then(|o| o.get(dep).copied())
            .as_ref()
        {
            return v;
        }
        let v = observe(dep);
        if let Ok(mut o) = self.observed.lock() {
            o.insert(dep.clone(), v);
        }
        v
    }
}

/// Whether the explicit fast mode is on.
fn trust_mtime() -> bool {
    std::env::var("CODEDNA_CACHE_TRUST_MTIME").is_ok_and(|v| !v.is_empty() && v != "0")
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
            Some(dir) => {
                Cache::open_with(&dir, &canonical, files, Versions::current(), trust_mtime())
            }
            None => Cache::off(),
        }
    }

    /// The cache for the repository at `canonical`, kept in `dir`, for
    /// values made by `versions`.
    fn open_with(
        dir: &Path,
        canonical: &Path,
        files: &[PathBuf],
        versions: Versions,
        trust_mtime: bool,
    ) -> Cache {
        let path = dir.join(format!(
            "{:016x}.json",
            fnv(canonical.to_string_lossy().as_bytes())
        ));
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let cutoff = u64::try_from(start.saturating_sub(RACY).as_nanos()).unwrap_or(u64::MAX);

        let seen: HashMap<PathBuf, Seen> = files
            .iter()
            .filter_map(|f| {
                Some((
                    normalize(f),
                    Seen {
                        stamp: stamp(f)?,
                        hash: OnceLock::new(),
                    },
                ))
            })
            .collect();

        let mut old = load(&path)
            .filter(|s| {
                s.format == FORMAT && s.codedna == env!("CARGO_PKG_VERSION") && s.root == canonical
            })
            .unwrap_or_default();
        let versions_changed = old.versions != versions;
        // Drop the values made by another version of their code.
        let (loc, layer, scan, imports, mentions) = (
            old.versions.loc != versions.loc,
            old.versions.layer != versions.layer,
            old.versions.scan != versions.scan,
            old.versions.imports != versions.imports,
            old.versions.mentions != versions.mentions,
        );
        for entry in old.files.values_mut() {
            if loc {
                entry.loc = None;
            }
            if layer {
                entry.layer = None;
            }
            if scan {
                entry.scan = None;
            }
            if imports {
                entry.imports = None;
            }
            if mentions {
                entry.mentions = None;
            }
        }
        old.root = canonical.to_path_buf();
        old.versions = versions;
        Cache {
            inner: Some(Inner {
                path,
                old,
                versions_changed,
                files: seen,
                cutoff,
                trust_mtime,
                dead: Mutex::new(None),
                new: Mutex::new(HashMap::new()),
                new_deps: Mutex::new(HashMap::new()),
                observed: Mutex::new(HashMap::new()),
                stats: Stats::default(),
            }),
        }
    }

    /// The cached value for `file` if its content is unchanged and `pick`
    /// still accepts it, else `compute()`, remembered for next time.
    fn get<T: Clone>(
        &self,
        file: &Path,
        kind: Kind,
        pick: impl Fn(&Inner, &Entry) -> Option<T>,
        compute: impl FnOnce() -> T,
        put: impl FnOnce(&mut Entry, T),
    ) -> T {
        let Some(inner) = &self.inner else {
            return compute();
        };
        let file = normalize(file);
        let Some(seen) = inner.files.get(&file) else {
            return compute();
        };
        let Some(hash) = inner.hash(&file, seen) else {
            return compute();
        };
        if let Some(old) = inner.old.files.get(&file) {
            if old.hash == hash {
                if let Some(value) = pick(inner, old) {
                    inner.stats.count(kind, true);
                    return value;
                }
            }
        }
        inner.stats.count(kind, false);
        let value = compute();
        let (size, mtime) = seen.stamp;
        if mtime < inner.cutoff {
            if let Ok(mut new) = inner.new.lock() {
                let entry = new.entry(file).or_insert_with(|| Entry {
                    size,
                    mtime,
                    hash,
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
            Kind::Loc,
            |_, e| e.loc.as_ref().map(|k| k.v.map(from)),
            compute,
            |e, v| e.loc = Some(Known { v: v.map(to) }),
        )
    }

    /// The layers `file`'s code shows, strongest first.
    pub fn layer(
        &self,
        file: &Path,
        compute: impl FnOnce() -> Option<Vec<(usize, CodeSign)>>,
    ) -> Option<Vec<(usize, CodeSign)>> {
        self.get(
            file,
            Kind::Layer,
            |_, e| e.layer.as_ref().map(|k| k.v.clone()),
            compute,
            |e, v| e.layer = Some(Known { v }),
        )
    }

    /// The JavaScript syntax scan of `file`.
    pub fn scan(&self, file: &Path, compute: impl FnOnce() -> JsScan) -> JsScan {
        self.get(
            file,
            Kind::Scan,
            |_, e| e.scan.clone(),
            compute,
            |e, v| e.scan = Some(v),
        )
    }

    /// The dependency facts of `file` and what happened to its imports.
    /// `observe` gives the current fingerprint of a lookup the resolution
    /// made; stored facts are reused only while every lookup they were
    /// worked out from gives the fingerprint it gave then.
    pub fn imports(
        &self,
        file: &Path,
        observe: impl Fn(&Dep) -> u64,
        compute: impl FnOnce() -> (Vec<DependencyFact>, ImportTally),
    ) -> (Vec<DependencyFact>, ImportTally) {
        let Some(inner) = &self.inner else {
            return compute();
        };
        let key = normalize(file);
        let (facts, tally, read) = self.get(
            file,
            Kind::Imports,
            |inner, e| {
                let i = e.imports.as_ref()?;
                for &index in &i.deps {
                    let (dep, then) = inner.old.deps.get(index as usize)?;
                    if inner.observe(dep, &observe) != *then {
                        return None;
                    }
                }
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
                Some((i.facts.clone(), tally, None))
            },
            || {
                let ((facts, tally), read) = deps::recording(compute);
                let mut read: Vec<(Dep, u64)> = read
                    .into_iter()
                    .map(|d| {
                        let v = inner.observe(&d, &observe);
                        (d, v)
                    })
                    .collect();
                read.sort();
                (facts, tally, Some(read))
            },
            |e, (facts, tally, _)| {
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
                    deps: Vec::new(),
                })
            },
        );
        if let Some(read) = read {
            if let Ok(mut new_deps) = inner.new_deps.lock() {
                new_deps.insert(key, read);
            }
        }
        (facts, tally)
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
            Kind::Mentions,
            |inner, e| {
                if Some(inner.old.dead) != dead {
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
        if std::env::var_os("CODEDNA_CACHE_STATS").is_some() {
            eprintln!("codedna cache stats: {}", inner.stats.json());
        }
        let new = inner.new.into_inner().unwrap_or_default();
        let new_deps = inner.new_deps.into_inner().unwrap_or_default();
        let dead = inner.dead.into_inner().ok().flatten();
        let mut old = inner.old;
        let dead_same = dead.is_none_or(|d| d == old.dead);
        let removed = old.files.keys().any(|f| !inner.files.contains_key(f));
        let restamped = inner.files.iter().any(|(file, seen)| {
            old.files
                .get(file)
                .is_some_and(|e| (e.size, e.mtime) != seen.stamp)
        });
        if new.is_empty() && !removed && !restamped && dead_same && !inner.versions_changed {
            return;
        }

        let mut table: Vec<(Dep, u64)> = Vec::new();
        let mut index: HashMap<(Dep, u64), u32> = HashMap::new();
        let mut intern = |dep: &(Dep, u64)| -> u32 {
            *index.entry(dep.clone()).or_insert_with(|| {
                table.push(dep.clone());
                u32::try_from(table.len() - 1).unwrap_or(u32::MAX)
            })
        };
        let mut files: HashMap<PathBuf, Entry> = HashMap::with_capacity(inner.files.len());
        for (file, seen) in &inner.files {
            let hash = seen.hash.get().copied().flatten();
            let mut entry = match old.files.remove(file) {
                // Read this run and unchanged: note the new stamp.
                Some(e) if hash == Some(e.hash) => Entry {
                    size: seen.stamp.0,
                    mtime: seen.stamp.1,
                    ..e
                },
                // Not read this run: keep it, to be checked when it is.
                Some(e) if hash.is_none() => e,
                _ => match hash {
                    Some(hash) => Entry {
                        size: seen.stamp.0,
                        mtime: seen.stamp.1,
                        hash,
                        ..Entry::default()
                    },
                    None => continue,
                },
            };
            if !dead_same {
                entry.mentions = None;
            }
            if let Some(imports) = entry.imports.as_mut() {
                let mut deps = Vec::with_capacity(imports.deps.len());
                for &i in &imports.deps {
                    match old.deps.get(i as usize) {
                        Some(dep) => deps.push(intern(dep)),
                        None => {
                            entry.imports = None;
                            break;
                        }
                    }
                }
                if let Some(imports) = entry.imports.as_mut() {
                    imports.deps = deps;
                }
            }
            if let Some(n) = new.get(file) {
                if n.hash != entry.hash {
                    continue;
                }
                entry.loc = n.loc.clone().or(entry.loc);
                entry.layer = n.layer.clone().or(entry.layer);
                entry.scan = n.scan.clone().or(entry.scan);
                entry.mentions = n.mentions.clone().or(entry.mentions);
                if let Some(mut imports) = n.imports.clone() {
                    imports.deps = new_deps
                        .get(file)
                        .map(|read| read.iter().map(&mut intern).collect())
                        .unwrap_or_default();
                    entry.imports = Some(imports);
                }
            }
            if !entry.is_empty() {
                files.insert(file.clone(), entry);
            }
        }
        let stored = Stored {
            format: FORMAT,
            codedna: env!("CARGO_PKG_VERSION").to_string(),
            root: old.root,
            versions: old.versions,
            dead: dead.unwrap_or(old.dead),
            deps: table,
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
        let assessed = dead_code_detector::assess_with(
            root,
            &dead,
            &files,
            &graph_map,
            &Default::default(),
            cache,
        );
        format!("{loc:?}\n{graph:?}\n{tallies:?}\n{assessed:?}")
    }

    fn cached(dir: &Path, root: &Path) -> Cache {
        cached_with(dir, root, Versions::current(), false)
    }

    fn cached_with(dir: &Path, root: &Path, versions: Versions, trust_mtime: bool) -> Cache {
        let files = scanner::scan_repository(root);
        Cache::open_with(
            dir,
            &fs::canonicalize(root).unwrap(),
            &files,
            versions,
            trust_mtime,
        )
    }

    /// Rewrite `path` with `text` of the same size, keeping its time.
    fn rewrite_in_place(path: &Path, text: &str) {
        let before = fs::metadata(path).unwrap();
        assert_eq!(before.len(), text.len() as u64);
        fs::write(path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(before.modified().unwrap())
            .unwrap();
    }

    fn counts(cache: &Cache, kind: &str) -> (u64, u64) {
        let stats = cache.inner.as_ref().unwrap().stats.json();
        (
            stats[kind]["reused"].as_u64().unwrap(),
            stats[kind]["computed"].as_u64().unwrap(),
        )
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
    fn a_same_size_edit_with_the_old_time_is_seen() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        let before = run(repo.path(), &first);
        first.save();
        // Same size and time, but now a comment.
        rewrite_in_place(&repo.path().join("src/a.ts"), "//port const a = 1;\n");
        let fresh = run(repo.path(), &Cache::off());
        assert_ne!(fresh, before);
        assert_eq!(run(repo.path(), &cached(store.path(), repo.path())), fresh);
    }

    #[test]
    fn trusting_timestamps_skips_reading_unchanged_files() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        let before = run(repo.path(), &first);
        first.save();
        rewrite_in_place(&repo.path().join("src/a.ts"), "//port const a = 1;\n");
        // The fast mode trusts size and time, so it misses the edit.
        let fast = cached_with(store.path(), repo.path(), Versions::current(), true);
        assert_eq!(run(repo.path(), &fast), before);
    }

    #[test]
    fn only_values_of_a_changed_version_are_recomputed() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        let first = cached(store.path(), repo.path());
        let fresh = run(repo.path(), &first);
        first.save();

        let same = cached(store.path(), repo.path());
        assert_eq!(run(repo.path(), &same), fresh);
        assert_eq!(counts(&same, "imports").1, 0);
        assert_eq!(counts(&same, "loc").1, 0);

        // A new resolver: imports are worked out again from the stored
        // scans; line counts stay.
        let resolver = Versions {
            imports: "next".into(),
            ..Versions::current()
        };
        let next = cached_with(store.path(), repo.path(), resolver.clone(), false);
        assert_eq!(run(repo.path(), &next), fresh);
        assert!(counts(&next, "imports").1 > 0);
        assert_eq!(counts(&next, "scan").1, 0);
        assert_eq!(counts(&next, "loc").1, 0);
        next.save();

        // A new parser: scans are made again too.
        let parser = Versions {
            scan: "next".into(),
            imports: "next 2".into(),
            ..resolver
        };
        let next = cached_with(store.path(), repo.path(), parser, false);
        assert_eq!(run(repo.path(), &next), fresh);
        assert!(counts(&next, "scan").1 > 0);
        assert_eq!(counts(&next, "loc").1, 0);
    }

    #[test]
    fn a_new_file_in_a_folder_recomputes_only_its_importers() {
        let repo = repo();
        let store = tempfile::tempdir().unwrap();
        fs::create_dir(repo.path().join("lib")).unwrap();
        put(&repo.path().join("lib/other.ts"), "import './x';\n", 1);
        let first = cached(store.path(), repo.path());
        run(repo.path(), &first);
        first.save();
        // `./x` now resolves; nothing in src/ looked in lib/.
        put(&repo.path().join("lib/x.ts"), "export {};\n", 1);
        let fresh = run(repo.path(), &Cache::off());
        let warm = cached(store.path(), repo.path());
        assert_eq!(run(repo.path(), &warm), fresh);
        let (reused, computed) = counts(&warm, "imports");
        assert_eq!(computed, 2, "lib/other.ts and the new lib/x.ts");
        assert_eq!(reused, 4);
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
