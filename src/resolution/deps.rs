//! What a file's resolved imports were worked out from.
//!
//! Import resolution reads more than the importing file: which other files
//! exist, the package and `tsconfig.json` chain above it, workspace
//! members, framework markers. While a file is resolved inside
//! [`recording`], every such lookup is noted as a [`Dep`]. The cache stores
//! the fingerprint of each one's value with the result and reuses the
//! result only while every fingerprint still matches, so an edit to one
//! package's config recomputes that package's files and nothing else.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// One piece of the repository a resolution looked at. While a file's
/// imports are resolved inside [`recording`], every lookup the resolver
/// makes is noted as a `Dep`; [`Resolver::observe`] later fingerprints the
/// current value of each, so a cached result is reused only while all the
/// pieces it read are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "at", rename_all = "snake_case")]
pub enum Dep {
    /// The files directly in a folder (each existence check).
    Dir(PathBuf),
    /// The importer's package, config and module format.
    Context(PathBuf),
    /// Every package with this name, and the workspaces declaring each.
    Name(String),
    /// The workspaces declaring the package in this folder.
    Members(PathBuf),
    /// The `tsconfig.json` of the package in this folder and the configs it
    /// references (`outDir` → `rootDir`).
    Built(PathBuf),
    /// The nearest `svelte.config.*` above this folder.
    Svelte(PathBuf),
    /// The nearest `docusaurus.config.*` above this folder.
    Docusaurus(PathBuf),
    /// The nearest `vite.config.*` above this folder.
    Vite(PathBuf),
    /// Every file below this folder (an `import.meta.glob` base).
    Glob(PathBuf),
    /// Whether this path is a file, a folder or missing on disk (Python and
    /// Rust module lookups).
    Path(PathBuf),
    /// The names of the repository's Python packages.
    PythonPackages,
}

thread_local! {
    static RECORDING: std::cell::RefCell<Option<HashSet<Dep>>> =
        const { std::cell::RefCell::new(None) };
}

/// Note a lookup, when a [`recording`] is running on this thread.
pub(crate) fn record(dep: impl FnOnce() -> Dep) {
    RECORDING.with(|r| {
        if let Some(deps) = r.borrow_mut().as_mut() {
            deps.insert(dep());
        }
    });
}

/// Run `f`, returning what it returns and every lookup the resolver made
/// on this thread while it ran.
pub fn recording<T>(f: impl FnOnce() -> T) -> (T, HashSet<Dep>) {
    let outer = RECORDING.with(|r| r.borrow_mut().replace(HashSet::new()));
    let value = f();
    let deps = RECORDING.with(|r| std::mem::replace(&mut *r.borrow_mut(), outer));
    let mut deps = deps.unwrap_or_default();
    // A nested recording's lookups belong to the outer one too.
    RECORDING.with(|r| {
        if let Some(outer) = r.borrow_mut().as_mut() {
            outer.extend(deps.iter().cloned());
        }
    });
    deps.shrink_to_fit();
    (value, deps)
}

/// A stable 64-bit fingerprint of a value's debug form. The resolver's
/// types keep maps as ordered lists, so the form does not depend on hash
/// order.
pub fn fingerprint(value: &impl std::fmt::Debug) -> u64 {
    xxhash_rust::xxh3::xxh3_64(format!("{value:?}").as_bytes())
}
