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
/// - Returns files only
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
