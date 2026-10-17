//! Hidden Unicode: bidirectional controls (Trojan Source) and invisible
//! characters.

use crate::audit::{finding, Finding, Severity, HIDDEN_UNICODE};

/// A bidirectional control anywhere on the line, or (in code) an invisible
/// character other than a byte order mark at the start of the file.
pub(crate) fn check_line(file: &str, index: usize, line: &str, code: bool) -> Option<Finding> {
    let n = Some(index + 1);
    if let Some(c) = line.chars().find(|c| is_bidi_control(*c)) {
        return Some(finding(
            if code { Severity::High } else { Severity::Low },
            HIDDEN_UNICODE,
            file,
            n,
            format!(
                "Hidden bidirectional control U+{:04X}: text can display differently from how it is read by the compiler (Trojan Source)",
                c as u32
            ),
        ));
    }
    if !code {
        return None;
    }
    let (_, c) = line
        .chars()
        .enumerate()
        .find(|&(col, c)| is_zero_width(c) && !(index == 0 && col == 0))?;
    Some(finding(
        Severity::Medium,
        HIDDEN_UNICODE,
        file,
        n,
        format!(
            "Invisible character U+{:04X}: two names that look the same can differ",
            c as u32
        ),
    ))
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Invisible characters with no business in source code. Joiners (U+200C,
/// U+200D) are left out: emoji sequences and some scripts need them.
fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{180E}')
}
