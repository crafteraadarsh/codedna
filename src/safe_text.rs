//! Terminal-safe text.
//!
//! File names and URLs come from repositories CodeDna does not trust. A name
//! containing escape sequences or bidirectional overrides could rewrite the
//! user's terminal or disguise what is printed, so such characters are
//! replaced before display.

/// Whether `c` could alter terminal output or text direction.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Whether `s` contains any character that is unsafe to print.
pub fn has_unsafe_chars(s: &str) -> bool {
    s.chars().any(is_unsafe_char)
}

/// Replace characters that are unsafe to print with `?`.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if is_unsafe_char(c) { '?' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_normal_text_alone() {
        assert_eq!(sanitize("src/main.rs"), "src/main.rs");
        assert_eq!(sanitize("café/日本.ts"), "café/日本.ts");
        assert!(!has_unsafe_chars("src/main.rs"));
    }

    #[test]
    fn replaces_escape_sequences() {
        assert_eq!(sanitize("a\u{1b}[2Jb"), "a?[2Jb");
        assert!(has_unsafe_chars("a\u{1b}b"));
    }

    #[test]
    fn replaces_newlines_and_bidi_overrides() {
        assert_eq!(sanitize("a\nb"), "a?b");
        assert_eq!(sanitize("evil\u{202E}txt.exe"), "evil?txt.exe");
        assert!(has_unsafe_chars("x\u{2066}y"));
    }
}
