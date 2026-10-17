//! Secrets: credentials matched by their documented format. A value is
//! never returned or printed, only the kind of credential.

use crate::architecture::Confidence;
use crate::audit::{finding, Finding, Severity, SECRET};

/// Secret findings on one line. `next` is the following line, where a
/// PEM key's body may start.
pub(crate) fn check_line(
    file: &str,
    n: Option<usize>,
    line: &str,
    next: Option<&str>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for kind in find_secrets(line) {
        if kind == "private key" && !pem_has_body(line, next) {
            continue;
        }
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
    out
}

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
pub(crate) const PEM_BEGIN: &str = "-----BEGIN ";
pub(crate) const PEM_PRIVATE: &str = "PRIVATE KEY-----";

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
