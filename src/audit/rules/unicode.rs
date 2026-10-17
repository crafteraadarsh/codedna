//! Hidden Unicode: bidirectional controls (Trojan Source) and invisible
//! characters. An invisible character in a name can make two names that
//! look the same differ; inside a string literal or a comment it changes
//! no name, so it is reported as weak evidence.

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
    let (at, c) = line
        .char_indices()
        .enumerate()
        .find(|&(col, (_, c))| is_zero_width(c) && !(index == 0 && col == 0))
        .map(|(_, ic)| ic)?;
    let message = format!(
        "Invisible character U+{:04X}: two names that look the same can differ",
        c as u32
    );
    Some(match place(line, at) {
        Some(place) => finding(Severity::Low, HIDDEN_UNICODE, file, n, message)
            .weak(format!("inside a {place}, not a name")),
        None => finding(Severity::Medium, HIDDEN_UNICODE, file, n, message)
            .signal("in code, outside strings and comments"),
    })
}

/// Whether byte `at` of the line sits in a string literal or a comment,
/// judged from the line alone. Strings spanning lines are not followed.
fn place(line: &str, at: usize) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') || t.starts_with("# ") {
        return Some("comment");
    }
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        if i >= at {
            return quote.map(|_| "string literal");
        }
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if matches!(c, '"' | '\'' | '`') {
                    quote = Some(c);
                } else if (c == '/' && prev == '/') || (c == '#' && prev.is_whitespace()) {
                    return Some("comment");
                }
            }
        }
        prev = c;
    }
    None
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Invisible characters with no business in source code. Joiners (U+200C,
/// U+200D) are left out: emoji sequences and some scripts need them.
fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}' | '\u{180E}')
}

#[cfg(test)]
mod tests {
    use super::place;

    #[test]
    fn strings_and_comments_are_told_apart_from_names() {
        let at = |line: &str| line.find('X').unwrap();
        assert_eq!(
            place("let a = \"aXb\";", at("let a = \"aXb\";")),
            Some("string literal")
        );
        assert_eq!(place("let aXb = 1;", at("let aXb = 1;")), None);
        assert_eq!(place("f('it\\'s', aXb)", at("f('it\\'s', aXb)")), None);
        assert_eq!(place("x = 1 // aXb", at("x = 1 // aXb")), Some("comment"));
        assert_eq!(place("  * aXb", at("  * aXb")), Some("comment"));
    }
}
