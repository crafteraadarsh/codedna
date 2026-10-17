//! Whether a credential is used, and by what. A key's folder explains it
//! but does not excuse it: a key under `src/test/resources` that the
//! application reads is as live as one in `src/`, while a module of fake
//! keys that only tests import is not.
//!
//! Only code counts as a user, and only a reference that resolves to the
//! key's file: its full path, its classpath name under `resources/`, its
//! path relative to the referring file, a JavaScript import that resolves
//! to it, or a Python module path. A file of the same name elsewhere, or a
//! config file mounting it, does not count. Aliased imports (`@/lib/keys`)
//! are not followed, so such a key keeps its lowered confidence.

use super::context::context_of;
use super::rules::CODE_EXTENSIONS;
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

fn extension(file: &str) -> &str {
    let name = file.rsplit('/').next().unwrap_or(file);
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

fn is_js(file: &str) -> bool {
    matches!(
        extension(file),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx"
    )
}

fn is_code(file: &str) -> bool {
    CODE_EXTENSIONS.contains(&extension(file))
}

fn plain(file: &str) -> &str {
    file.trim_start_matches("./")
}

/// The path of `to` as written from inside the folder holding `from`.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').collect();
    let from = &from[..from.len().saturating_sub(1)];
    let to: Vec<&str> = to.split('/').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/")
}

fn quoted(text: &str, s: &str) -> bool {
    ['"', '\'', '`']
        .iter()
        .any(|q| text.contains(&format!("{q}{s}{q}")))
}

/// Whether `text`, the content of code file `referrer`, refers to
/// `candidate`.
fn refers(candidate: &str, referrer: &str, text: &str) -> bool {
    if text.contains(candidate) {
        return true;
    }
    if let Some(i) = candidate.rfind("/resources/") {
        let name = &candidate[i + "/resources/".len()..];
        if quoted(text, name) || quoted(text, &format!("/{name}")) {
            return true;
        }
    }
    let rel = relative(referrer, candidate);
    if is_js(candidate) {
        if !is_js(referrer) {
            return false;
        }
        let stem = rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s);
        let stem = if stem.starts_with("../") {
            stem.to_string()
        } else {
            format!("./{stem}")
        };
        let mut forms = vec![stem.clone()];
        if let Some(dir) = stem.strip_suffix("/index") {
            forms.push(dir.to_string());
        }
        return forms.iter().any(|f| {
            ["", ".js", ".mjs", ".cjs", ".ts", ".jsx", ".tsx"]
                .iter()
                .any(|e| quoted(text, &format!("{f}{e}")))
        });
    }
    if extension(candidate) == "py" {
        let module = candidate.trim_end_matches(".py").replace('/', ".");
        let module = module.strip_prefix("src.").unwrap_or(&module);
        return text.contains(module);
    }
    if is_code(candidate) {
        return false;
    }
    if rel.contains('/') {
        text.contains(&rel)
    } else {
        quoted(text, &rel) || text.contains(&format!("./{rel}"))
    }
}

/// Re-weigh secret findings by whether the project's own code uses the
/// file they are in.
pub(super) fn weigh_secrets(root: &Path, files: &[PathBuf], findings: &mut [Finding]) {
    // Files with a key whose evidence is not already weak (placeholders).
    let candidates: BTreeSet<String> = findings
        .iter()
        .filter(|f| f.check == SECRET && f.evidence.strength != Strength::Weak)
        .filter(|f| made_up_context(f.context) || (f.context.is_none() && is_js(&f.file)))
        .map(|f| plain(&f.file).to_string())
        .collect();
    if candidates.is_empty() {
        return;
    }

    let hits: Vec<(String, String)> = files
        .par_iter()
        .flat_map_iter(|path| {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let relative = plain(&crate::safe_text::sanitize(&relative)).to_string();
            let readable = is_code(&relative)
                && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_REFERRER);
            let text = readable
                .then(|| std::fs::read(path).ok())
                .flatten()
                .filter(|b| !b[..b.len().min(8_192)].contains(&0))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            candidates
                .iter()
                .filter(|c| !text.is_empty() && **c != relative && refers(c, &relative, &text))
                .map(|c| (c.clone(), relative.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    let mut referrers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (file, by) in &hits {
        referrers.entry(file).or_default().push(by);
    }

    for f in findings.iter_mut() {
        if f.check != SECRET || !candidates.contains(plain(&f.file)) {
            continue;
        }
        let mut by = referrers.get(plain(&f.file)).cloned().unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_climb_out_of_the_referrers_folder() {
        assert_eq!(relative("src/a/b.ts", "src/a/mock.ts"), "mock.ts");
        assert_eq!(relative("src/a/b.ts", "src/c/mock.ts"), "../c/mock.ts");
        assert_eq!(relative("app.py", "tests/keys.py"), "tests/keys.py");
    }

    #[test]
    fn only_references_that_resolve_to_the_file_count() {
        let mock = "packages/a/src/mock.ts";
        assert!(refers(mock, "packages/a/src/x.test.ts", "from './mock.js'"));
        assert!(!refers(mock, "packages/b/src/index.ts", "from './mock.js'"));
        assert!(refers(mock, "packages/a/x.ts", "from \"./src/mock\""));
        let pem = "src/test/resources/server.pem";
        assert!(refers(pem, "src/main/A.java", "load(\"server.pem\")"));
        assert!(!refers(
            pem,
            "src/main/A.java",
            "load(\"other/server.pem\")"
        ));
        assert!(refers(
            "tests/keys.py",
            "app/sign.py",
            "from tests.keys import K"
        ));
    }
}
