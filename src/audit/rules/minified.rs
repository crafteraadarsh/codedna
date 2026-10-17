//! Very long lines: minified or generated code that cannot be reviewed.

use crate::audit::{finding, Finding, Severity, MINIFIED};

/// Lines at least this long are reported as minified.
pub(crate) const LONG_LINE: usize = 5_000;

/// A long line in the project's own code, unless it is number data.
pub(crate) fn check_line(file: &str, n: Option<usize>, line: &str) -> Option<Finding> {
    (line.len() >= LONG_LINE && !is_number_data(line)).then(|| long_line(file, n, line))
}

pub(crate) fn long_line(file: &str, line: Option<usize>, text: &str) -> Finding {
    finding(
        Severity::Low,
        MINIFIED,
        file,
        line,
        format!(
            "Line of {} characters: minified or generated code is hard to review",
            text.len()
        ),
    )
}

/// Mostly numbers, such as SVG path data or a lookup table.
fn is_number_data(line: &str) -> bool {
    let numeric = line
        .bytes()
        .filter(|b| {
            b.is_ascii_digit() || b.is_ascii_whitespace() || matches!(b, b'.' | b',' | b'-')
        })
        .count();
    numeric * 10 >= line.len() * 9
}
