//! Language parsers that turn source text into dependency references.
//!
//! Each parser reads one file and reports what it found together with how
//! well the file parsed, so the report can say which facts rest on a full
//! syntax tree and which files only got a best-effort text scan.

pub mod javascript;

use serde::{Deserialize, Serialize};

/// How far a file got through its language's parser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    /// Not a file CodeDna parses with a syntax parser (Python, Rust, ...).
    #[default]
    NotParsed,
    /// Parsed with no syntax errors.
    Parsed,
    /// Parsed with syntax errors the parser recovered from; facts come from
    /// the recovered tree.
    Partial,
    /// The parser gave up; only possibilities from a text scan are reported.
    Failed,
}

impl ParseStatus {
    /// The worse of two statuses, for a file made of several script blocks.
    pub fn worst(self, other: ParseStatus) -> ParseStatus {
        fn rank(status: ParseStatus) -> u8 {
            match status {
                ParseStatus::NotParsed => 0,
                ParseStatus::Parsed => 1,
                ParseStatus::Partial => 2,
                ParseStatus::Failed => 3,
            }
        }
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

/// One syntax error, for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    /// Line number, from 1.
    pub line: usize,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worst_status_wins() {
        use ParseStatus::*;
        assert_eq!(NotParsed.worst(Parsed), Parsed);
        assert_eq!(Parsed.worst(Partial), Partial);
        assert_eq!(Failed.worst(Partial), Failed);
        assert_eq!(Parsed.worst(NotParsed), Parsed);
    }
}
