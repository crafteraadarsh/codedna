//! Obfuscation: decoded strings run as code, hex-escaped strings and long
//! encoded blobs. Encoded data alone is weak evidence; only decoding that
//! feeds a call which runs code is reported as a strong signal. Nothing is
//! ever decoded or run.

use crate::audit::{finding, Finding, Severity, OBFUSCATION};

/// Base64 runs at least this long are reported as encoded blobs.
const BASE64_RUN: usize = 400;
/// Strings with at least this many `\x` escapes are reported.
const HEX_ESCAPES: usize = 40;

/// Obfuscation on one line of the project's own code. The encoded-blob
/// check runs once per file (`blob_reported`); `blob_on_line` says whether
/// it fired on this line.
pub(crate) fn check_line(
    file: &str,
    n: Option<usize>,
    line: &str,
    secret_on_line: bool,
    blob_reported: &mut bool,
    blob_on_line: &mut bool,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if EXEC_CALLS.iter().any(|t| line.contains(t)) && DECODERS.iter().any(|t| line.contains(t)) {
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

    if !*blob_reported && !secret_on_line {
        let run = longest_base64_run(line);
        if run.len() >= BASE64_RUN && !run.starts_with(WASM_BASE64) {
            let run = run.len();
            *blob_reported = true;
            *blob_on_line = true;
            out.push(finding(
                Severity::Low,
                OBFUSCATION,
                file,
                n,
                format!("Encoded blob of {run} characters in source code"),
            ));
        }
    }
    out
}

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
