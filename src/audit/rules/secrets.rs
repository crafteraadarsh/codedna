//! Secrets: credentials matched by their documented format. A value is
//! never returned or printed, only the kind of credential.
//!
//! The format alone is strong evidence. It is weakened, never dropped, by
//! fingerprints of made-up values: the examples published in a vendor's
//! docs, sequential filler such as `1a2b3c…`, values cut short with `...`
//! and keys a nearby comment calls throwaway. Entropy is not used: a
//! random-looking value says nothing about whether it is live.

use crate::architecture::Confidence;
use crate::audit::{finding, Finding, Severity, SECRET};

/// Secret findings on one line. `next` is the following line, where a
/// PEM key's body may start; `before` holds the few lines above it.
pub(crate) fn check_line(
    file: &str,
    n: Option<usize>,
    line: &str,
    next: Option<&str>,
    before: &[&str],
) -> Vec<Finding> {
    let mut out = Vec::new();
    let throwaway = throwaway_comment(line, before);
    for Match { kind, made_up } in matches(line) {
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
            out.push(f.weak("Firebase web keys are public by design"));
            continue;
        }
        let mut f = finding(
            Severity::High,
            SECRET,
            file,
            n,
            format!("Possible {kind} (value not shown). Revoke it if it is real."),
        )
        .signal(format!("format: {kind}"));
        if let Some(why) = made_up {
            f.weaken(why);
        } else if let Some(marker) = throwaway {
            f.weaken(format!("a comment nearby says \"{marker}\""));
        }
        out.push(f);
    }
    out
}

/// Words in a comment that mark a key as made up.
const THROWAWAY: &[&str] = &[
    "throwaway",
    "dummy",
    "fake key",
    "fake token",
    "fake secret",
    "test key",
    "test token",
    "test-only",
    "testing only",
    "not a real",
    "not real",
    "example key",
    "sample key",
    "do not use in production",
];

/// The marker, when a comment on the line or the lines just above it says
/// the key is made up.
fn throwaway_comment(line: &str, before: &[&str]) -> Option<&'static str> {
    std::iter::once(line)
        .chain(before.iter().copied())
        .filter_map(comment_of)
        .find_map(|c| {
            let c = c.to_ascii_lowercase();
            THROWAWAY.iter().copied().find(|m| c.contains(m))
        })
}

/// The comment part of a line, if it has one.
fn comment_of(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t.starts_with('#') || t.starts_with('*') || t.starts_with("/*") || t.starts_with("<!--") {
        return Some(t);
    }
    [" //", " #", "/*"]
        .iter()
        .filter_map(|m| line.find(m).map(|i| &line[i..]))
        .next()
        .or_else(|| t.starts_with("//").then_some(t))
}

/// One key on a line, and why it looks made up, if it does.
struct Match {
    kind: &'static str,
    made_up: Option<&'static str>,
}

/// Values vendors publish in their docs as examples. Kept split so that
/// they never appear whole in the source.
const PUBLISHED_EXAMPLES: &[(&str, &str)] = &[
    // GitHub's "Creating a personal access token" docs.
    ("ghp_", "16C7e42F292c6912E7710c838347Ae178B4a"),
];

/// Why a token body looks made up, if it does.
fn made_up(prefix: &str, body: &str, after: &str) -> Option<&'static str> {
    if PUBLISHED_EXAMPLES
        .iter()
        .any(|(p, b)| *p == prefix && *b == body)
    {
        return Some("a published documentation example");
    }
    if after.starts_with("...") || after.starts_with('\u{2026}') {
        return Some("the value is cut short with ...");
    }
    if sequential_filler(body) {
        return Some("sequential filler, not a generated value");
    }
    None
}

/// Whether the letters and the digits of `body` each count up, as in
/// `1A2b3C4D…` or `abc123def456`, which no key generator produces.
fn sequential_filler(body: &str) -> bool {
    let letters: Vec<u8> = body
        .bytes()
        .filter(u8::is_ascii_alphabetic)
        .map(|b| b.to_ascii_lowercase())
        .collect();
    let digits: Vec<u8> = body.bytes().filter(u8::is_ascii_digit).collect();
    let counts_up = |seq: &[u8], next: fn(u8) -> u8| {
        let steps = seq.windows(2).filter(|w| w[1] == next(w[0])).count();
        seq.len() >= 6 && steps * 5 >= (seq.len() - 1) * 4
    };
    counts_up(&letters, |b| if b == b'z' { b'a' } else { b + 1 })
        && counts_up(&digits, |b| if b == b'9' { b'0' } else { b + 1 })
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
    matches(line).into_iter().map(|m| m.kind).collect()
}

fn matches(line: &str) -> Vec<Match> {
    let mut kinds: Vec<Match> = Vec::new();
    let bytes = line.as_bytes();
    for rule in SECRET_RULES {
        if kinds.iter().any(|m| m.kind == rule.kind) {
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
            let body = &line[body_start..body_start + body_len];
            if is_placeholder(body) {
                continue;
            }
            kinds.push(Match {
                kind: rule.kind,
                made_up: made_up(rule.prefix, body, &line[body_start + body_len..]),
            });
            break;
        }
    }
    if line.contains(PEM_BEGIN) && line.contains(PEM_PRIVATE) {
        kinds.push(Match {
            kind: "private key",
            made_up: None,
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counting_filler_is_made_up_and_generated_values_are_not() {
        assert!(sequential_filler("1A2b3C4D5e6F7g8H9i0J"));
        assert!(sequential_filler("abcdefghij0123456789"));
        assert!(!sequential_filler("R8vKq2Lm9TzX4wPj7NcY1bHs6DfGa3Ue5Qo0"));
        // Too short to tell.
        assert!(!sequential_filler("a1b2c3"));
    }

    #[test]
    fn a_throwaway_comment_counts_only_as_a_comment() {
        assert_eq!(
            throwaway_comment("x = 1", &["// dummy key for local runs"]),
            Some("dummy")
        );
        assert_eq!(throwaway_comment("const dummyValue = 1;", &[]), None);
        assert_eq!(
            throwaway_comment("key = 'v' # not a real key", &[]),
            Some("not a real")
        );
    }
}
