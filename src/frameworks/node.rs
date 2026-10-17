//! `package.json` entry points: the files `main`, `module`, `browser`,
//! `bin` and `exports` name, and the files the package's `scripts` (and
//! Prisma's `prisma.seed`) run.

use super::{relative, Adapters, Package, Repo};
use crate::facts::Certainty;
use std::path::PathBuf;

pub const VERSION: u32 = 1;
const ENTRY_RULE: &str = "node.package_entry@v1";
const SCRIPT_RULE: &str = "node.script@v1";

/// Fields naming the files a package exposes.
const ENTRY_FIELDS: &[&str] = &["main", "module", "browser", "bin", "exports"];

/// Extensions of files a script can run directly.
const SCRIPT_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts", "tsx", "jsx", "py"];

pub fn detect(repo: &Repo, package: &Package, out: &mut Adapters) {
    let before = out.entries.len();
    for field in ENTRY_FIELDS {
        let Some(value) = package.json.get(field) else {
            continue;
        };
        let line = key_line(&package.text, &format!("\"{field}\""), 0);
        let named = if *field == "exports" {
            export_targets(value)
        } else {
            let mut named = Vec::new();
            collect_strings(value, &mut named);
            named.into_iter().map(|path| (path, None)).collect()
        };
        let blocked = blocked_exports(package.json.get("exports"));
        for (path, key) in named {
            for target in targets(repo, package, &path) {
                // Node refuses targets outside the package.
                if target == package.manifest || !target.starts_with(&package.dir) {
                    continue;
                }
                if let Some(key) = &key {
                    if is_blocked(key, &path, &relative(&package.dir, &target), &blocked) {
                        continue;
                    }
                }
                out.entries
                    .push(package.entry(&target, ENTRY_RULE, Certainty::Declaration, line));
            }
        }
    }

    let mut commands = Vec::new();
    if let Some(scripts) = package.json.get("scripts") {
        collect_strings(scripts, &mut commands);
    }
    if let Some(seed) = package.json.pointer("/prisma/seed") {
        collect_strings(seed, &mut commands);
    }
    let scripts_at = package.text.find("\"scripts\"").unwrap_or(0);
    // Only words that name a source file: `eslint src` must not make all of
    // `src/` an entry point.
    for word in commands
        .iter()
        .flat_map(|c| c.split(|ch: char| ch.is_whitespace() || ch == '='))
        .map(|word| word.trim_matches(|ch| ch == '"' || ch == '\''))
    {
        let runs_file = std::path::Path::new(word)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e));
        if !runs_file || word.contains('*') {
            continue;
        }
        if let Some(target) = repo.resolve(&package.dir, word) {
            let line = key_line(&package.text, word, scripts_at);
            out.entries
                .push(package.entry(&target, SCRIPT_RULE, Certainty::Inferred, line));
        }
    }

    if out.entries.len() > before {
        let line = ENTRY_FIELDS
            .iter()
            .chain(&["scripts"])
            .filter_map(|key| key_line(&package.text, &format!("\"{key}\""), 0))
            .min();
        out.active
            .push(package.adapter("node", VERSION, line, Vec::new()));
    }
}

/// The package files a manifest path names. A single `*` (as in
/// `"./plugins/*": "./src/plugins/*.ts"`) matches the package's files.
fn targets(repo: &Repo, package: &Package, path: &str) -> Vec<PathBuf> {
    let path = path.trim();
    let Some((prefix, suffix)) = path.split_once('*') else {
        return repo.resolve(&package.dir, path).into_iter().collect();
    };
    let prefix = prefix.trim_start_matches("./");
    // `"./*": "./*"` opens the whole package to importers; it names no
    // entry point.
    if suffix.contains('*') || path.starts_with('/') || prefix.is_empty() {
        return Vec::new();
    }
    package
        .files
        .iter()
        .filter(|file| {
            let rel = relative(&package.dir, file);
            rel.len() > prefix.len() + suffix.len()
                && rel.starts_with(prefix)
                && rel.ends_with(suffix)
                && (suffix.contains('.') || super::is_js(file))
        })
        .cloned()
        .collect()
}

/// Every target of an `exports` value with the subpath key it belongs to:
/// `{"./a": {"import": "./a.js"}}` gives `("./a.js", Some("./a"))`.
fn export_targets(value: &serde_json::Value) -> Vec<(String, Option<String>)> {
    let subpaths = value
        .as_object()
        .filter(|map| map.keys().all(|k| k.starts_with('.')));
    match subpaths {
        Some(map) => map
            .iter()
            .flat_map(|(key, value)| {
                let mut named = Vec::new();
                collect_strings(value, &mut named);
                named.into_iter().map(move |path| (path, Some(key.clone())))
            })
            .collect(),
        None => {
            let mut named = Vec::new();
            collect_strings(value, &mut named);
            named.into_iter().map(|path| (path, None)).collect()
        }
    }
}

/// The `exports` subpaths mapped to `null`, which Node refuses.
fn blocked_exports(exports: Option<&serde_json::Value>) -> Vec<String> {
    exports
        .and_then(|e| e.as_object())
        .map(|map| {
            map.iter()
                .filter(|(_, v)| v.is_null())
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `file`, reached through the pattern `key` -> `target`, has a
/// subpath that a `null` export blocks.
fn is_blocked(key: &str, target: &str, file: &str, blocked: &[String]) -> bool {
    if blocked.is_empty() {
        return false;
    }
    let subpath = match (
        key.split_once('*'),
        target.trim().trim_start_matches("./").split_once('*'),
    ) {
        (Some((kp, ks)), Some((tp, ts))) => {
            match file.strip_prefix(tp).and_then(|rest| rest.strip_suffix(ts)) {
                Some(matched) => format!("{kp}{matched}{ks}"),
                None => return false,
            }
        }
        _ => key.to_string(),
    };
    blocked.iter().any(|b| match b.split_once('*') {
        Some((bp, bs)) => subpath.starts_with(bp) && subpath.ends_with(bs),
        None => *b == subpath,
    })
}

/// The line of the first `needle` at or after byte `from`.
fn key_line(text: &str, needle: &str, from: usize) -> Option<usize> {
    let from = from.min(text.len());
    let at = from + text[from..].find(needle)?;
    Some(text[..at].matches('\n').count() + 1)
}

/// Every string inside a JSON value: `"bin": {"a": "./cli.js"}` and nested
/// conditional `exports`.
fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}
