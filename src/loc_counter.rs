//! LOC counting module.
//!
//! Counts non-empty lines for text files and skips binary/unreadable files.

use std::fs;
use std::path::Path;

/// Count non-empty lines in a file.
///
/// Returns:
/// - `Some(count)` if the file is readable and treated as text
/// - `None` if the file is unreadable or considered binary
pub fn count_lines(file_path: &Path) -> Option<usize> {
    let bytes = fs::read(file_path).ok()?;

    if is_binary(&bytes) {
        return None;
    }

    let content = std::str::from_utf8(&bytes).ok()?;

    let count = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    Some(count)
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
}
