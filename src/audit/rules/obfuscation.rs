//! Obfuscation: decoded strings run as code, hex-escaped strings and long
//! encoded blobs. Encoded data alone is weak evidence; only decoding that
//! feeds a call which runs code is reported as a strong signal. Nothing is
//! ever decoded or run.

use crate::audit::{finding, Finding, Severity, OBFUSCATION};

/// Base64 runs at least this long are reported as encoded blobs.
const BASE64_RUN: usize = 400;
/// Strings with at least this many `\x` escapes are reported.
const HEX_ESCAPES: usize = 40;
/// How far after a code runner's `(` its closing parenthesis is looked for.
const ARGUMENT_SCAN: usize = 2_000;

/// Obfuscation on one line of the project's own code. The encoded-blob
/// check runs once per file (`blob_reported`); `blob_on_line` says whether
/// it fired on this line. `bundle` says the file looks generated.
pub(crate) fn check_line(
    file: &str,
    n: Option<usize>,
    line: &str,
    bundle: bool,
    secret_on_line: bool,
    blob_reported: &mut bool,
    blob_on_line: &mut bool,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some((call, decoder)) = decoded_into_runner(line) {
        out.push(
            finding(
                Severity::High,
                OBFUSCATION,
                file,
                n,
                "Decodes data and runs it as code on the same line: a common way to hide what code does".to_string(),
            )
            .signal(format!("decoder {decoder} inside the arguments of {call}")),
        );
    } else if EXEC_CALLS.iter().any(|t| line.contains(t))
        && DECODERS.iter().any(|t| line.contains(t))
    {
        out.push(
            finding(
                Severity::Low,
                OBFUSCATION,
                file,
                n,
                "Decodes data and calls a code runner on the same line, but the decoded data is not passed to it".to_string(),
            )
            .weak("no data flow from the decoder into the code runner"),
        );
    }

    let escapes = line.matches("\\x").count();
    if escapes >= HEX_ESCAPES {
        let f = finding(
            Severity::Medium,
            OBFUSCATION,
            file,
            n,
            format!("String built from {escapes} hex escapes: its content is hidden"),
        );
        out.push(if bundle {
            f.weak("in a generated bundle")
        } else {
            f
        });
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

/// The code runner and decoder when a decoder sits inside the arguments of
/// a call that runs code, as in `eval(atob(x))`. The arguments are found
/// by matching parentheses within a bounded distance; strings holding
/// parentheses can mislead it, which errs towards a weak finding.
fn decoded_into_runner(line: &str) -> Option<(&'static str, &'static str)> {
    let bytes = line.as_bytes();
    for call in EXEC_CALLS {
        for (start, _) in line.match_indices(call) {
            let standalone = start == 0 || {
                let b = bytes[start - 1];
                !(b.is_ascii_alphanumeric() || b == b'_')
            };
            if !standalone {
                continue;
            }
            let open = start + call.len();
            let end = (open + ARGUMENT_SCAN).min(bytes.len());
            let mut depth = 1usize;
            let mut close = None;
            for (i, &b) in bytes[open..end].iter().enumerate() {
                match b {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(open + i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else { continue };
            let args = &line[open..close];
            if let Some(decoder) = DECODERS.iter().find(|d| args.contains(*d)) {
                return Some((call.trim_end_matches('('), decoder.trim_end_matches('(')));
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::decoded_into_runner;

    #[test]
    fn only_a_decoder_inside_the_runner_counts() {
        assert_eq!(decoded_into_runner("eval(atob(p))"), Some(("eval", "atob")));
        assert_eq!(
            decoded_into_runner("new Function(Buffer.from(s, 'base64').toString())()"),
            Some(("Function", "base64"))
        );
        assert_eq!(decoded_into_runner("const d = atob(p); eval(x)"), None);
        assert_eq!(decoded_into_runner("/;base64,(.+)$/.exec(s)"), None);
        // A longer name that ends in a runner is not one.
        assert_eq!(decoded_into_runner("myeval(atob(p))"), None);
        // No closing parenthesis within reach.
        assert_eq!(decoded_into_runner("eval(atob(p"), None);
    }
}
