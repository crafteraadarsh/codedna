//! Where a finding sits: tests, docs, examples, fixtures, vendored
//! libraries and translations. Context explains and ranks a finding; it is
//! not by itself proof that the finding is harmless.

use super::{Finding, Severity, HIDDEN_UNICODE, MINIFIED, OBFUSCATION};
use crate::architecture::Confidence;
use std::path::Path;

/// Folders whose code users of the project do not run, or did not write:
/// copies of third-party code and translated text.
const LOWERED_DIRS: &[(&str, &str)] = &[
    ("test", "test"),
    ("tests", "test"),
    ("__tests__", "test"),
    ("__mocks__", "test"),
    ("spec", "test"),
    ("e2e", "test"),
    ("testdata", "fixture"),
    ("fixtures", "fixture"),
    ("fixture", "fixture"),
    ("docs", "docs"),
    ("doc", "docs"),
    ("examples", "example"),
    ("example", "example"),
    ("samples", "example"),
    ("sample", "example"),
    ("demo", "example"),
    ("demos", "example"),
    ("mock", "fixture"),
    ("mocks", "fixture"),
    ("mock-api", "fixture"),
    ("mock-data", "fixture"),
    ("__fixtures__", "fixture"),
    ("vendor", "vendored"),
    ("vendors", "vendored"),
    ("third_party", "vendored"),
    ("third-party", "vendored"),
    (".yarn", "vendored"),
    ("locales", "translation"),
    ("locale", "translation"),
    ("i18n", "translation"),
    ("l10n", "translation"),
    ("translations", "translation"),
];

/// `Some(kind)` when `file` is a test, doc, example, fixture, vendored
/// library or translation.
pub(super) fn context_of(file: &str) -> Option<&'static str> {
    let lower = file.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('/').collect();
    let name = parts.pop().unwrap_or("");
    if let Some((_, kind)) = parts
        .iter()
        .find_map(|p| LOWERED_DIRS.iter().find(|(dir, _)| dir == p))
    {
        return Some(kind);
    }
    if crate::architecture::is_test(Path::new(file)) || name == "conftest.py" {
        return Some("test");
    }
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    if matches!(
        ext,
        "md" | "mdx" | "markdown" | "markdoc" | "rst" | "adoc" | "txt"
    ) && !name.starts_with("requirements")
    {
        return Some("docs");
    }
    if name.contains(".example") || name.contains(".sample") {
        return Some("example");
    }
    // Prebuilt copies of libraries: `jquery.min.js`, `swagger-ui-bundle.js`.
    if [".min.", "-min.", ".bundle.", "-bundle."]
        .iter()
        .any(|marker| name.contains(marker))
    {
        return Some("vendored");
    }
    None
}

/// Findings that tests and fixtures produce on purpose: long expected
/// output, encoded sample data and odd characters in sample files.
pub(super) fn is_fixture_data(f: &Finding) -> bool {
    f.check == MINIFIED
        || (f.severity == Severity::Low && matches!(f.check, OBFUSCATION | HIDDEN_UNICODE))
}

/// Lower a finding one step on severity and confidence.
pub(super) fn lower(f: &mut Finding, context: &'static str) {
    f.severity = match f.severity {
        Severity::High => Severity::Medium,
        _ => Severity::Low,
    };
    f.confidence = match f.confidence {
        Confidence::High => Confidence::Medium,
        _ => Confidence::Low,
    };
    f.context = Some(context);
    f.evidence
        .signals
        .push(format!("in a {context} file: lowered one step"));
}
