//! Whether a credential is used, and by what. A key's folder explains it
//! but does not excuse it: a key under `src/test/resources` that the
//! application reads is as live as one in `src/`, while a module of fake
//! keys that only tests import is not.
//!
//! Usage is found by name: other files that mention a key file's name, or
//! (for JavaScript and TypeScript) import its module. Only files holding a
//! candidate key are looked up, and only when there is one.

use super::context::context_of;
use super::{Finding, Strength, SECRET};
use crate::architecture::Confidence;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Files larger than this are not searched for references.
const MAX_REFERRER: u64 = 1024 * 1024;
/// Referrers named in a signal.
const NAMED: usize = 2;

/// Contexts whose keys are usually made up for the occasion.
fn made_up_context(context: Option<&str>) -> bool {
    matches!(context, Some("test" | "fixture" | "example"))
}

fn is_js(file: &str) -> bool {
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    matches!(
        ext,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx"
    )
}

/// Strings in another file that mean it uses `file`.
fn needles(file: &str) -> Vec<String> {
    let name = file.rsplit('/').next().unwrap_or(file);
    let mut out = vec![name.to_string()];
    if is_js(file) {
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        for q in ['\'', '"'] {
            out.push(format!("/{stem}{q}"));
            out.push(format!("/{stem}.js{q}"));
        }
    }
    out
}

/// Re-weigh secret findings by who uses the file they are in.
pub(super) fn weigh_secrets(root: &Path, files: &[PathBuf], findings: &mut [Finding]) {
    // Files with a key whose evidence is not already weak (placeholders).
    let candidates: BTreeSet<String> = findings
        .iter()
        .filter(|f| f.check == SECRET && f.evidence.strength != Strength::Weak)
        .filter(|f| made_up_context(f.context) || (f.context.is_none() && is_js(&f.file)))
        .map(|f| f.file.clone())
        .collect();
    if candidates.is_empty() {
        return;
    }
    let wanted: Vec<(String, Vec<String>)> =
        candidates.iter().map(|c| (c.clone(), needles(c))).collect();

    let hits: Vec<(String, String)> = files
        .par_iter()
        .flat_map_iter(|path| {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let readable = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_REFERRER);
            let text = readable
                .then(|| std::fs::read(path).ok())
                .flatten()
                .filter(|b| !b[..b.len().min(8_192)].contains(&0))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            wanted
                .iter()
                .filter(|(file, needles)| {
                    *file != crate::safe_text::sanitize(&relative)
                        && needles.iter().any(|n| text.contains(n.as_str()))
                })
                .map(|(file, _)| (file.clone(), relative.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut referrers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (file, by) in &hits {
        referrers.entry(file).or_default().push(by);
    }

    for f in findings.iter_mut() {
        if f.check != SECRET || !candidates.contains(&f.file) {
            continue;
        }
        let mut by = referrers.get(f.file.as_str()).cloned().unwrap_or_default();
        by.sort_unstable();
        let (product, other): (Vec<&str>, Vec<&str>) =
            by.iter().partition(|r| context_of(r).is_none());
        let named = |list: &[&str]| {
            let mut s = list
                .iter()
                .take(NAMED)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            if list.len() > NAMED {
                s.push_str(&format!(" and {} more", list.len() - NAMED));
            }
            s
        };
        if !product.is_empty() {
            f.evidence.signals.push(format!(
                "used by the project's own code: {}",
                named(&product)
            ));
            // Lowered for where it sits, but the project uses it: keep
            // the confidence the key format earns.
            if made_up_context(f.context) {
                f.confidence = Confidence::High;
            }
        } else if made_up_context(f.context) {
            if other.is_empty() {
                f.weaken("not used by the project's own code");
            } else {
                f.weaken(format!("used only by tests or examples: {}", named(&other)));
            }
        } else if !other.is_empty() {
            f.weaken(format!(
                "imported only by tests or examples: {}",
                named(&other)
            ));
        }
    }
}
