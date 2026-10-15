//! LOC counting module.
//!
//! Splits each text file into code, comment and blank lines, and skips
//! binary or unreadable files.  "Lines of code" everywhere in CodeDna means
//! code lines only: comments and blank lines are reported separately.

use std::fs;
use std::path::Path;

/// Line counts for one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCounts {
    /// Lines holding code (a line with code and a trailing comment is code).
    pub code: usize,
    /// Lines holding only a comment.
    pub comment: usize,
    /// Empty or whitespace-only lines.
    pub blank: usize,
}

/// Comment syntax for a family of languages.
struct Syntax {
    line: &'static [&'static str],
    block: &'static [(&'static str, &'static str)],
    /// Characters that open and close a string literal.  Comment markers
    /// inside a string are not comments (`"http://x"`, `"src/*.rs"`).
    quotes: &'static [char],
}

const C_LIKE: Syntax = Syntax {
    line: &["//"],
    block: &[("/*", "*/")],
    quotes: &['"', '\'', '`'],
};
/// Rust: `'a` is a lifetime, not a string, so single quotes are not tracked.
const RUST: Syntax = Syntax {
    line: &["//"],
    block: &[("/*", "*/")],
    quotes: &['"'],
};
const CSS: Syntax = Syntax {
    line: &[],
    block: &[("/*", "*/")],
    quotes: &['"', '\''],
};
const HASH: Syntax = Syntax {
    line: &["#"],
    block: &[],
    quotes: &['"', '\''],
};
const PHP: Syntax = Syntax {
    line: &["//", "#"],
    block: &[("/*", "*/")],
    quotes: &['"', '\''],
};
const DASH: Syntax = Syntax {
    line: &["--"],
    block: &[("/*", "*/")],
    quotes: &['"', '\''],
};
const LUA: Syntax = Syntax {
    line: &["--"],
    block: &[("--[[", "]]")],
    quotes: &['"', '\''],
};
const HASKELL: Syntax = Syntax {
    line: &["--"],
    block: &[("{-", "-}")],
    quotes: &['"'],
};
const MARKUP: Syntax = Syntax {
    line: &[],
    block: &[("<!--", "-->")],
    quotes: &[],
};
const VUE: Syntax = Syntax {
    line: &["//"],
    block: &[("<!--", "-->"), ("/*", "*/")],
    quotes: &['"', '\'', '`'],
};
const LISP: Syntax = Syntax {
    line: &[";"],
    block: &[],
    quotes: &['"'],
};
const ERLANG: Syntax = Syntax {
    line: &["%"],
    block: &[],
    quotes: &['"'],
};

/// Comment syntax for a file, from its extension or name.  `None` means the
/// format has no comments (JSON, Markdown, plain text): every non-blank line
/// counts as code.
fn syntax_for(path: &Path) -> Option<&'static Syntax> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "dockerfile" | "makefile" | "gemfile" | "rakefile" | "procfile" | "justfile"
    ) || name.starts_with("dockerfile.")
    {
        return Some(&HASH);
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => &RUST,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "java" | "kt" | "kts"
        | "go" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "cs" | "swift" | "scala"
        | "dart" | "groovy" | "gradle" | "scss" | "less" | "zig" | "proto" | "sol" | "m" | "mm"
        | "fs" | "jsonc" | "json5" => &C_LIKE,
        "css" => &CSS,
        "vue" | "svelte" | "astro" => &VUE,
        "php" => &PHP,
        "py" | "pyi" | "rb" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "pl" | "pm"
        | "r" | "yml" | "yaml" | "toml" | "ini" | "cfg" | "conf" | "tf" | "hcl" | "ex" | "exs"
        | "nim" | "cmake" | "dockerfile" | "mk" | "jl" | "cr" | "nix" | "graphql" | "gql" => &HASH,
        "sql" => &DASH,
        "lua" => &LUA,
        "hs" | "elm" => &HASKELL,
        "html" | "htm" | "xml" | "svg" | "xhtml" => &MARKUP,
        "clj" | "cljs" | "cljc" | "edn" | "lisp" | "el" | "scm" => &LISP,
        "erl" | "hrl" => &ERLANG,
        _ => return None,
    })
}

/// Count code lines in a file (comments and blank lines excluded).
///
/// Returns:
/// - `Some(count)` if the file is readable and treated as text
/// - `None` if the file is unreadable or considered binary
#[cfg(test)]
pub fn count_lines(file_path: &Path) -> Option<usize> {
    count_line_kinds(file_path).map(|c| c.code)
}

/// Split a file into code, comment and blank lines.
///
/// Returns `None` for unreadable, binary or non-UTF-8 files.
pub fn count_line_kinds(file_path: &Path) -> Option<LineCounts> {
    let bytes = fs::read(file_path).ok()?;

    if is_binary(&bytes) {
        return None;
    }

    let content = std::str::from_utf8(&bytes).ok()?;
    Some(classify(content, syntax_for(file_path)))
}

/// Classify every line of `content`.
fn classify(content: &str, syntax: Option<&Syntax>) -> LineCounts {
    let mut counts = LineCounts::default();
    let Some(syntax) = syntax else {
        for line in content.lines() {
            if line.trim().is_empty() {
                counts.blank += 1;
            } else {
                counts.code += 1;
            }
        }
        return counts;
    };

    // Bytes the loop below must look at one by one: whitespace, quotes and
    // the first byte of each comment marker. Runs of other ASCII bytes are
    // plain code and are skipped in one step.
    let mut special = [false; 256];
    for b in [b' ', b'\t', b'\r', b'\n', 0x0b, 0x0c] {
        special[b as usize] = true;
    }
    for q in syntax.quotes {
        let mut buf = [0; 4];
        special[q.encode_utf8(&mut buf).as_bytes()[0] as usize] = true;
    }
    for marker in syntax
        .line
        .iter()
        .chain(syntax.block.iter().map(|(open, _)| open))
    {
        if let Some(&b) = marker.as_bytes().first() {
            special[b as usize] = true;
        }
    }

    // The closing marker of the block comment we are inside, if any.
    let mut in_block: Option<&str> = None;
    for line in content.lines() {
        if line.trim().is_empty() {
            counts.blank += 1;
            continue;
        }
        let mut has_code = false;
        let mut has_comment = false;
        let mut rest = line;
        let mut quote: Option<char> = None;
        while !rest.is_empty() {
            if let Some(close) = in_block {
                has_comment = true;
                match rest.find(close) {
                    Some(i) => {
                        rest = &rest[i + close.len()..];
                        in_block = None;
                    }
                    None => rest = "",
                }
                continue;
            }
            if let Some(q) = quote {
                let mut chars = rest.char_indices();
                let mut end = rest.len();
                while let Some((i, c)) = chars.next() {
                    if c == '\\' {
                        chars.next();
                    } else if c == q {
                        end = i + c.len_utf8();
                        quote = None;
                        break;
                    }
                }
                rest = &rest[end..];
                continue;
            }
            let plain = rest
                .bytes()
                .position(|b| b >= 0x80 || special[b as usize])
                .unwrap_or(rest.len());
            if plain > 0 {
                has_code = true;
                rest = &rest[plain..];
                continue;
            }
            let c = rest.chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                rest = &rest[c.len_utf8()..];
                continue;
            }
            if let Some(&(open, close)) = syntax.block.iter().find(|(o, _)| rest.starts_with(o)) {
                has_comment = true;
                in_block = Some(close);
                rest = &rest[open.len()..];
                continue;
            }
            if syntax.line.iter().any(|m| rest.starts_with(m)) {
                has_comment = true;
                break;
            }
            has_code = true;
            if syntax.quotes.contains(&c) {
                quote = Some(c);
            }
            rest = &rest[c.len_utf8()..];
        }
        if has_code {
            counts.code += 1;
        } else if has_comment {
            counts.comment += 1;
        } else {
            counts.code += 1;
        }
    }
    counts
}

/// Heuristic binary detection:
/// - contains null bytes
/// - or contains invalid UTF-8 (checked by caller via from_utf8)
fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();

        let path = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    #[test]
    fn counts_only_non_empty_lines() {
        let dir = unique_temp_dir("codedna_loc_counter_text");
        let file_path = dir.join("sample.txt");

        let content = "first line\n\n   \nsecond line\n\t\nthird line\n";
        fs::write(&file_path, content).expect("failed to write sample text");

        let result = count_lines(&file_path);
        assert_eq!(result, Some(3));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn returns_none_for_binary_file_with_null_bytes() {
        let dir = unique_temp_dir("codedna_loc_counter_binary");
        let file_path = dir.join("binary.bin");

        let mut file = File::create(&file_path).expect("failed to create binary file");
        file.write_all(&[0x00, 0xFF, 0x10, 0x41])
            .expect("failed to write binary bytes");

        let result = count_lines(&file_path);
        assert_eq!(result, None);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn returns_none_for_unreadable_or_missing_file() {
        let missing = PathBuf::from("/this/path/should/not/exist/codedna_missing.txt");
        let result = count_lines(&missing);
        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_for_invalid_utf8_text_like_file() {
        let dir = unique_temp_dir("codedna_loc_counter_invalid_utf8");
        let file_path = dir.join("invalid_utf8.dat");

        // Invalid UTF-8 sequence without null bytes.
        fs::write(&file_path, vec![0xF0, 0x28, 0x8C, 0x28]).expect("failed to write bytes");

        let result = count_lines(&file_path);
        assert_eq!(result, None);

        fs::remove_dir_all(dir).ok();
    }

    fn kinds(src: &str, name: &str) -> LineCounts {
        classify(src, syntax_for(Path::new(name)))
    }

    #[test]
    fn separates_code_comments_and_blank_lines() {
        let src = "// header\nfn main() {\n\n    /* block\n       more */\n    let x = 1; // trailing\n}\n";
        assert_eq!(
            kinds(src, "main.rs"),
            LineCounts {
                code: 3,
                comment: 3,
                blank: 1
            }
        );
    }

    #[test]
    fn comment_markers_inside_strings_are_code() {
        let src = "const url = \"http://example.com\";\nconst glob = \"src/*.ts\";\nconst y = 2;\n";
        assert_eq!(kinds(src, "a.ts").code, 3);
        assert_eq!(kinds(src, "a.ts").comment, 0);
    }

    #[test]
    fn rust_lifetimes_do_not_open_strings() {
        let src = "fn f<'a>(x: &'a str) {}\n// note\n";
        assert_eq!(
            kinds(src, "lib.rs"),
            LineCounts {
                code: 1,
                comment: 1,
                blank: 0
            }
        );
    }

    #[test]
    fn hash_comments_and_markup() {
        assert_eq!(
            kinds("# c\nx = 1\n", "a.py"),
            LineCounts {
                code: 1,
                comment: 1,
                blank: 0
            }
        );
        assert_eq!(kinds("<!-- a\nb -->\n<p>x</p>\n", "a.html").comment, 2);
        assert_eq!(kinds("# Title\ntext\n", "README.md").code, 2);
    }
}
