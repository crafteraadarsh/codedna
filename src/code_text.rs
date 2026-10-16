//! Source text with comments and string contents blanked out.
//!
//! Evidence such as `@RestController` or `pgTable(` means something only in
//! code: the same words inside a string, a comment or a test fixture say
//! nothing about what a file does. [`code_only`] returns the text with every
//! comment and every string literal's contents replaced by spaces, keeping
//! the quotes, line breaks and byte offsets, so a match in the result is code
//! and its line number is the line in the original file.

/// Comment and string rules for one family of languages.
struct Syntax {
    /// `//` and `/* */` comments.
    c_comments: bool,
    /// `#` comments (`#[` and `#!` stay code where they are attributes).
    hash_comments: bool,
    /// `'…'` is a string, not a character literal or lifetime.
    single_quote_strings: bool,
    /// `` `…` `` is a string (JavaScript templates, Go raw strings).
    backtick_strings: bool,
    /// `"""…"""` and `'''…'''` blocks.
    triple_quotes: bool,
    /// Rust `r"…"` and `r#"…"#`.
    rust_raw: bool,
    /// C# `@"…"`, where `""` is a quote.
    verbatim: bool,
}

fn syntax(ext: &str) -> Option<Syntax> {
    let base = Syntax {
        c_comments: true,
        hash_comments: false,
        single_quote_strings: false,
        backtick_strings: false,
        triple_quotes: false,
        rust_raw: false,
        verbatim: false,
    };
    Some(match ext {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" | "vue" | "svelte"
        | "astro" => Syntax {
            single_quote_strings: true,
            backtick_strings: true,
            ..base
        },
        "go" => Syntax {
            backtick_strings: true,
            ..base
        },
        "rs" => Syntax {
            rust_raw: true,
            ..base
        },
        "cs" => Syntax {
            verbatim: true,
            ..base
        },
        "java" | "kt" | "kts" | "scala" | "swift" | "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" => {
            Syntax {
                triple_quotes: matches!(ext, "kt" | "kts" | "scala" | "swift"),
                ..base
            }
        }
        "dart" => Syntax {
            single_quote_strings: true,
            triple_quotes: true,
            ..base
        },
        "php" => Syntax {
            hash_comments: true,
            single_quote_strings: true,
            ..base
        },
        "py" | "pyi" => Syntax {
            c_comments: false,
            hash_comments: true,
            single_quote_strings: true,
            triple_quotes: true,
            ..base
        },
        "rb" => Syntax {
            c_comments: false,
            hash_comments: true,
            single_quote_strings: true,
            ..base
        },
        _ => return None,
    })
}

/// `text` with comments and string contents replaced by spaces, for a file
/// with extension `ext` (lowercase, no dot). Languages without known rules
/// come back unchanged.
pub fn code_only(ext: &str, text: &str) -> String {
    let Some(syntax) = syntax(ext) else {
        return text.to_string();
    };
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        let (start, end) = if syntax.c_comments && rest.starts_with(b"//") {
            (i, line_end(bytes, i))
        } else if syntax.c_comments && rest.starts_with(b"/*") {
            (i, find(bytes, i + 2, b"*/").map_or(bytes.len(), |e| e + 2))
        } else if syntax.hash_comments
            && rest[0] == b'#'
            && !rest.starts_with(b"#[")
            && !(i == 0 && rest.starts_with(b"#!"))
        {
            (i, line_end(bytes, i))
        } else if syntax.triple_quotes && (rest.starts_with(b"\"\"\"") || rest.starts_with(b"'''"))
        {
            let close = &rest[..3];
            let end = find(bytes, i + 3, close).unwrap_or(bytes.len());
            blank(&mut out, i + 3, end);
            i = (end + 3).min(bytes.len());
            continue;
        } else if syntax.rust_raw && is_rust_raw(bytes, i) {
            let hashes = rest[1..].iter().take_while(|b| **b == b'#').count();
            let open = i + 1 + hashes + 1;
            let mut close = vec![b'"'];
            close.extend(std::iter::repeat_n(b'#', hashes));
            let end = find(bytes, open, &close).unwrap_or(bytes.len());
            blank(&mut out, open, end);
            i = (end + close.len()).min(bytes.len());
            continue;
        } else if syntax.verbatim && rest.starts_with(b"@\"") {
            let mut j = i + 2;
            while j < bytes.len() {
                if bytes[j] == b'"' {
                    if bytes.get(j + 1) == Some(&b'"') {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            blank(&mut out, i + 2, j);
            i = j + 1;
            continue;
        } else if rest[0] == b'"'
            || (rest[0] == b'`' && syntax.backtick_strings)
            || (rest[0] == b'\'' && (syntax.single_quote_strings || is_char_literal(rest)))
        {
            let quote = rest[0];
            let end = string_end(bytes, i + 1, quote, quote == b'`' && ext == "go");
            blank(&mut out, i + 1, end);
            i = end + 1;
            continue;
        } else {
            i += 1;
            continue;
        };
        blank(&mut out, start, end);
        i = end;
    }
    // Only ASCII bytes were replaced, and only whole runs between ASCII
    // delimiters, so the result is still valid UTF-8.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Replace `out[start..end]` with spaces, keeping line breaks.
fn blank(out: &mut [u8], start: usize, end: usize) {
    let end = end.min(out.len());
    for b in &mut out[start.min(end)..end] {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |p| from + p)
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Index of the quote closing a string that opened just before `from`.
/// Ordinary strings end at a line break; raw ones (Go backticks, JavaScript
/// templates) may span lines.
fn string_end(bytes: &[u8], from: usize, quote: u8, raw: bool) -> usize {
    let multiline = quote == b'`';
    let mut j = from;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' if !raw => j += 2,
            b'\n' if !multiline => return j,
            b if b == quote => return j,
            _ => j += 1,
        }
    }
    bytes.len()
}

/// `'a'`, `'\n'`, `'\u{1F600}'`: a character literal rather than a
/// lifetime (`'a`) or a stray apostrophe.
fn is_char_literal(rest: &[u8]) -> bool {
    match rest.get(1) {
        Some(b'\\') => rest[2..].iter().take(12).any(|b| *b == b'\''),
        Some(b'\'') | None => false,
        Some(_) => {
            // One character, possibly several UTF-8 bytes, then a quote.
            let len = std::str::from_utf8(&rest[1..rest.len().min(5)])
                .ok()
                .or_else(|| std::str::from_utf8(&rest[1..rest.len().min(2)]).ok())
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8);
            rest.get(1 + len) == Some(&b'\'')
        }
    }
}

/// `r"` or `r#…#"` not preceded by an identifier character.
fn is_rust_raw(bytes: &[u8], i: usize) -> bool {
    if bytes[i] != b'r' {
        return false;
    }
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        return false;
    }
    let hashes = bytes[i + 1..].iter().take_while(|b| **b == b'#').count();
    bytes.get(i + 1 + hashes) == Some(&b'"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanks_strings_and_comments_keeping_offsets() {
        let ts = "const s = \"@RestController\"; // pgTable(\nexport const t = pgTable('t', {});\n/* @Injectable( */ x";
        let code = code_only("ts", ts);
        assert_eq!(code.len(), ts.len());
        assert_eq!(code.lines().count(), ts.lines().count());
        assert!(!code.contains("@RestController"));
        assert_eq!(code.matches("pgTable(").count(), 1);
        assert!(code.lines().nth(1).unwrap().contains("pgTable("));
        assert!(!code.contains("@Injectable("));
    }

    #[test]
    fn handles_language_specific_literals() {
        let py = "x = '''\n@app.route('/a')\n'''\n# models.Model\nclass A(models.Model): pass\n";
        let code = code_only("py", py);
        assert!(!code.contains("@app.route"));
        assert_eq!(code.matches("(models.Model)").count(), 1);

        let rs = "let s = r#\"diesel::table!\"#; fn f<'a>(x: &'a str) -> char { 'x' }\ndiesel::table! {}";
        let code = code_only("rs", rs);
        assert_eq!(code.matches("diesel::table!").count(), 1);
        assert!(code.contains("fn f<'a>(x: &'a str)"), "{code}");

        let cs = "var s = @\"[ApiController] \"\"q\"\"\"; [ApiController] class C {}";
        let code = code_only("cs", cs);
        assert_eq!(code.matches("[ApiController]").count(), 1);

        let go = "var s = `func(next http.Handler) http.Handler`\n";
        assert!(!code_only("go", go).contains("http.Handler"));

        let rb = "# < ApplicationRecord\nclass U < ApplicationRecord; end";
        assert_eq!(
            code_only("rb", rb).matches("< ApplicationRecord").count(),
            1
        );
    }

    #[test]
    fn escapes_attributes_and_unknown_languages() {
        let js = "const a = \"say \\\"@Service\\\"\"; @Service";
        assert_eq!(code_only("js", js).matches("@Service").count(), 1);
        let rs = "#[derive(Queryable)]\nstruct U;";
        assert!(code_only("rs", rs).contains("#[derive(Queryable"));
        let php = "<?php\n#[Route('/x')]\n# CREATE TABLE\n";
        assert!(code_only("php", php).contains("#[Route("));
        assert!(!code_only("php", php).contains("CREATE TABLE"));
        assert_eq!(code_only("sql", "CREATE TABLE t;"), "CREATE TABLE t;");
    }

    #[test]
    fn keeps_utf8_valid() {
        let ts = "const s = \"héllo — ✓\"; // ünïcode\nlet c = 1;";
        let code = code_only("ts", ts);
        assert_eq!(code.len(), ts.len());
        assert!(code.contains("let c = 1;"));
    }
}
