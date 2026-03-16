//! Repository map module.
//!
//! Renders a directory tree of the repository using box-drawing characters,
//! respecting the same ignore rules as `scanner.rs`.
//!
//! Example output:
//! ```text
//! src/
//! ├── components/
//! │   ├── Button.tsx
//! │   └── Navbar.tsx
//! ├── hooks/
//! │   └── useAuth.ts
//! └── utils/
//!     └── helpers.ts
//! ```

use std::ffi::OsStr;
use std::fmt::Write as FmtWrite;
use std::path::Path;

/// Directories that are always excluded from the tree (mirrors `scanner.rs`).
const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "coverage",
];

/// Default maximum depth when none is specified.
pub const DEFAULT_MAX_DEPTH: usize = 6;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render the directory tree rooted at `root` up to `max_depth` levels deep.
///
/// Returns the full tree as a `String` ready for printing to stdout.
///
/// Directories are rendered before files at each level; both groups are
/// sorted alphabetically.  Ignored directories are silently skipped.
pub fn render_tree(root: &Path, max_depth: usize) -> String {
    let mut out = String::new();

    // Print the root label.
    let root_label = root.file_name().and_then(OsStr::to_str).unwrap_or(".");

    let _ = writeln!(out, "{root_label}/");

    render_dir(root, "", max_depth, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Recursive renderer
// ---------------------------------------------------------------------------

fn render_dir(dir: &Path, prefix: &str, depth_remaining: usize, out: &mut String) {
    if depth_remaining == 0 {
        let _ = writeln!(out, "{prefix}    …");
        return;
    }

    // Collect entries, filtering ignored directories.
    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name_str = name.to_str().unwrap_or("");
                // Only filter *directories* that are in the ignore list.
                // Hidden files / hidden non-ignored dirs are still shown.
                let is_ignored_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && IGNORED_DIRS.contains(&name_str);
                !is_ignored_dir
            })
            .collect(),
        Err(_) => return,
    };

    // Sort: directories first (alphabetically), then files (alphabetically).
    entries.sort_by(|a, b| {
        let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a
                .file_name()
                .to_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b.file_name().to_str().unwrap_or("").to_lowercase()),
        }
    });

    let count = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let extension = if is_last { "    " } else { "│   " };

        let name = entry.file_name();
        let name_str = name.to_str().unwrap_or("?");
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            let _ = writeln!(out, "{prefix}{connector}{name_str}/");
            let new_prefix = format!("{prefix}{extension}");
            render_dir(&entry.path(), &new_prefix, depth_remaining - 1, out);
        } else {
            let _ = writeln!(out, "{prefix}{connector}{name_str}");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap();
    }

    // -----------------------------------------------------------------------
    // render_tree
    // -----------------------------------------------------------------------

    #[test]
    fn renders_root_label_at_top() {
        let dir = unique_temp_dir("codedna_map_root");
        touch(&dir.join("main.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);
        let first_line = tree.lines().next().unwrap_or("");

        // First line should end with '/' and contain the dir name.
        assert!(first_line.ends_with('/'), "root label should end with '/'");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn renders_files_in_root() {
        let dir = unique_temp_dir("codedna_map_files");
        touch(&dir.join("main.rs"));
        touch(&dir.join("lib.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        assert!(tree.contains("main.rs"));
        assert!(tree.contains("lib.rs"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn renders_subdirectory_with_trailing_slash() {
        let dir = unique_temp_dir("codedna_map_subdir");
        fs::create_dir_all(dir.join("src")).unwrap();
        touch(&dir.join("src/main.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        assert!(tree.contains("src/"));
        assert!(tree.contains("main.rs"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn directories_appear_before_files() {
        let dir = unique_temp_dir("codedna_map_order");
        fs::create_dir_all(dir.join("src")).unwrap();
        touch(&dir.join("readme.md"));
        touch(&dir.join("src/main.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);
        let lines: Vec<&str> = tree.lines().collect();

        let src_pos = lines.iter().position(|l| l.contains("src/")).unwrap();
        let readme_pos = lines.iter().position(|l| l.contains("readme.md")).unwrap();

        assert!(
            src_pos < readme_pos,
            "directories should appear before files\n{tree}"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_ignored_directories() {
        let dir = unique_temp_dir("codedna_map_ignored");
        for ignored in IGNORED_DIRS {
            let d = dir.join(ignored);
            fs::create_dir_all(&d).unwrap();
            touch(&d.join("should_not_appear.txt"));
        }
        touch(&dir.join("main.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        for ignored in IGNORED_DIRS {
            assert!(
                !tree.contains(ignored),
                "ignored dir '{ignored}' appeared in tree:\n{tree}"
            );
        }
        assert!(tree.contains("main.rs"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn respects_max_depth() {
        let dir = unique_temp_dir("codedna_map_depth");
        // Create a/b/c/deep.rs — 3 levels deep
        fs::create_dir_all(dir.join("a/b/c")).unwrap();
        touch(&dir.join("a/b/c/deep.rs"));

        // At depth 2 we should see a/ and b/ but not c/ contents
        let tree = render_tree(&dir, 2);

        assert!(tree.contains("a/"));
        assert!(tree.contains("b/"));
        // deep.rs should NOT appear (depth exceeded)
        assert!(
            !tree.contains("deep.rs"),
            "deep.rs should be hidden at depth 2:\n{tree}"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn uses_correct_box_drawing_chars() {
        let dir = unique_temp_dir("codedna_map_box");
        touch(&dir.join("aaa.rs"));
        touch(&dir.join("bbb.rs"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        // Last entry should use └──, earlier ones ├──
        assert!(tree.contains("├── "), "should contain ├── :\n{tree}");
        assert!(tree.contains("└── "), "should contain └── :\n{tree}");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn last_entry_uses_corner_connector() {
        let dir = unique_temp_dir("codedna_map_corner");
        touch(&dir.join("aaa.rs")); // first
        touch(&dir.join("zzz.rs")); // last (alphabetically)

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);
        let lines: Vec<&str> = tree.lines().collect();

        let last_entry = lines.last().unwrap();
        assert!(
            last_entry.contains("└── "),
            "last entry should use └── :\n{tree}"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn handles_empty_directory() {
        let dir = unique_temp_dir("codedna_map_empty");
        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        // Only the root label should appear
        assert_eq!(tree.lines().count(), 1);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn handles_nested_directories_with_continuation_pipe() {
        let dir = unique_temp_dir("codedna_map_pipe");
        fs::create_dir_all(dir.join("src/components")).unwrap();
        fs::create_dir_all(dir.join("src/hooks")).unwrap();
        touch(&dir.join("src/components/Button.tsx"));
        touch(&dir.join("src/hooks/useAuth.ts"));

        let tree = render_tree(&dir, DEFAULT_MAX_DEPTH);

        // The continuation pipe │ should appear between sibling dirs
        assert!(
            tree.contains("│"),
            "continuation pipe │ should appear:\n{tree}"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn handles_missing_directory_gracefully() {
        let missing = PathBuf::from("/no/such/path/codedna_map_missing");
        let tree = render_tree(&missing, DEFAULT_MAX_DEPTH);
        // Should not panic; just produces the root label
        assert!(!tree.is_empty());
    }
}
