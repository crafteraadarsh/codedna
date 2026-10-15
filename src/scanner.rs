use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "coverage",
];

/// Files larger than this are skipped.  Source files are far smaller; a huge
/// file is almost always generated or a data dump, and reading it would only
/// cost memory and time.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

fn is_ignored_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .map(|name| IGNORED_DIRS.contains(&name))
            .unwrap_or(false)
}

/// Recursively scans a repository and returns filtered file paths.
///
/// Rules:
/// - Traverses recursively from `root`
/// - Skips directories in `IGNORED_DIRS`
/// - Returns regular files only; symbolic links are not followed, so a
///   repository cannot point the scan at files outside itself
/// - Skips files larger than `MAX_FILE_BYTES`
/// - Skips paths with control or bidirectional-override characters, which
///   could otherwise rewrite the terminal when the path is printed
/// - Silently skips unreadable entries
pub fn scan_repository(root: &Path) -> Vec<PathBuf> {
    if !root.exists() || !root.is_dir() {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !is_ignored_dir(entry))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .metadata()
                .map(|m| m.len() <= MAX_FILE_BYTES)
                .unwrap_or(false)
        })
        .filter(|entry| {
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
            !crate::safe_text::has_unsafe_chars(&relative.to_string_lossy())
        })
        .map(|entry| entry.into_path())
        .collect();

    files.sort_unstable();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
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
    fn scans_files_recursively_and_ignores_noise_dirs() {
        let root = unique_temp_dir("codedna_scanner_test");

        // Included files
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        let keep_file = src_dir.join("main.rs");
        File::create(&keep_file).unwrap();

        let nested_dir = src_dir.join("nested");
        fs::create_dir_all(&nested_dir).unwrap();
        let keep_nested_file = nested_dir.join("lib.rs");
        File::create(&keep_nested_file).unwrap();

        // Ignored directories and files
        for ignored in IGNORED_DIRS {
            let d = root.join(ignored);
            fs::create_dir_all(&d).unwrap();
            File::create(d.join("ignored.txt")).unwrap();
        }

        let mut result = scan_repository(&root);

        // Normalize expected order
        result.sort_unstable();

        assert!(result.contains(&keep_file));
        assert!(result.contains(&keep_nested_file));

        for ignored in IGNORED_DIRS {
            let ignored_file = root.join(ignored).join("ignored.txt");
            assert!(!result.contains(&ignored_file));
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn skips_files_over_the_size_limit() {
        let root = unique_temp_dir("codedna_scanner_size_test");
        let small = root.join("small.rs");
        File::create(&small).unwrap();
        let huge = root.join("huge.json");
        // Sparse file: takes no real disk space.
        File::create(&huge)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();

        let result = scan_repository(&root);

        assert!(result.contains(&small));
        assert!(!result.contains(&huge));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    #[cfg(unix)]
    fn skips_paths_with_control_characters() {
        let root = unique_temp_dir("codedna_scanner_ctrl_test");
        let normal = root.join("main.rs");
        File::create(&normal).unwrap();
        let evil = root.join("evil\u{1b}[2J.rs");
        File::create(&evil).unwrap();

        let result = scan_repository(&root);

        assert!(result.contains(&normal));
        assert!(!result.contains(&evil));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    #[cfg(unix)]
    fn does_not_follow_symlinks_out_of_the_repository() {
        let outside = unique_temp_dir("codedna_scanner_outside");
        let secret = outside.join("secret.env");
        fs::write(&secret, "TOKEN=abc").unwrap();

        let root = unique_temp_dir("codedna_scanner_symlink_test");
        std::os::unix::fs::symlink(&secret, root.join("link.env")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linkdir")).unwrap();

        let result = scan_repository(&root);

        assert!(result.is_empty(), "symlinks were followed: {result:?}");

        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(outside).ok();
    }

    #[test]
    fn returns_empty_for_missing_or_non_directory_path() {
        let missing = PathBuf::from("/this/path/should/not/exist/codedna_scanner");
        assert!(scan_repository(&missing).is_empty());

        let root = unique_temp_dir("codedna_scanner_file_path_test");
        let file = root.join("single.txt");
        File::create(&file).unwrap();
        assert!(scan_repository(&file).is_empty());

        fs::remove_dir_all(root).ok();
    }
}
