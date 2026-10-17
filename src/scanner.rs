use ignore::{DirEntry, WalkBuilder};
use std::path::{Path, PathBuf};

/// Directories never scanned, wherever they are.
const ALWAYS_IGNORED: &[&str] = &["node_modules", ".git"];

/// Directory names that usually hold build output. They are skipped only
/// where they really are output (see [`is_output_dir`]); elsewhere, such as
/// `src/core/build`, they are ordinary source folders.
const OUTPUT_NAMES: &[&str] = &["target", "dist", "build", "coverage"];

/// Files that mark a project root: build output sits next to them.
const PROJECT_MARKERS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "composer.json",
    "Gemfile",
    "deno.json",
    "deno.jsonc",
];

/// Files larger than this are skipped.  Source files are far smaller; a huge
/// file is almost always generated or a data dump, and reading it would only
/// cost memory and time.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

fn is_ignored_dir(root: &Path, entry: &DirEntry) -> bool {
    // In a git worktree or submodule, `.git` is a small file pointing at the
    // real repository. It is git metadata, not part of the project.
    if entry.file_name() == ".git" {
        return true;
    }
    entry.file_type().is_some_and(|t| t.is_dir()) && is_skipped_dir(root, entry.path())
}

/// Whether the directory `dir` inside the scanned `root` is left out:
/// `node_modules` and `.git` always, a `build`, `dist`, `target` or
/// `coverage` folder only where [`is_output_dir`] says it holds output.
pub fn is_skipped_dir(root: &Path, dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if ALWAYS_IGNORED.contains(&name) {
        return true;
    }
    OUTPUT_NAMES.contains(&name) && is_output_dir(root, dir)
}

/// Whether a folder with an output-like name holds build output:
///
/// - it sits at the top of the scan, or next to a project manifest
///   (`package.json`, `Cargo.toml`, `pyproject.toml`, …), where build
///   tools write; or
/// - its package declares it as output: `package.json` `main`, `module`,
///   `types`, `browser`, `bin`, `exports` or `files` point into it, or
///   `tsconfig.json` names it as `outDir`.
///
/// Anywhere else, such as `packages/astro/src/core/build`, the folder is
/// source. Folders matched by `.gitignore` are skipped by the walk anyway.
fn is_output_dir(root: &Path, dir: &Path) -> bool {
    let Some(parent) = dir.parent() else {
        return true;
    };
    if parent == root || PROJECT_MARKERS.iter().any(|m| parent.join(m).is_file()) {
        return true;
    }
    // Look for a declaration in the enclosing package, up to the scan root.
    let mut current = parent;
    loop {
        let relative = dir.strip_prefix(current).unwrap_or(dir);
        let relative = relative.to_string_lossy().replace('\\', "/");
        if let Some(out) = tsconfig_out_dir(&current.join("tsconfig.json")) {
            if declares(&out, &relative) {
                return true;
            }
        }
        let manifest = current.join("package.json");
        if manifest.is_file() {
            return package_outputs(&manifest)
                .iter()
                .any(|path| declares(path, &relative));
        }
        if current == root {
            return false;
        }
        match current.parent() {
            Some(up) if up.starts_with(root) => current = up,
            _ => return false,
        }
    }
}

/// Whether the declared path `declared` (relative to its package) is the
/// folder `relative` or lies inside it. Globs count up to their first
/// wildcard.
fn declares(declared: &str, relative: &str) -> bool {
    if declared.starts_with('!') {
        return false;
    }
    let cut = declared
        .find(['*', '?', '[', '{'])
        .unwrap_or(declared.len());
    let path = declared[..cut].replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    let path = path.trim_end_matches('/');
    !path.is_empty()
        && (path == relative
            || path
                .strip_prefix(relative)
                .is_some_and(|rest| rest.starts_with('/')))
}

/// Paths a `package.json` names as what it publishes or runs.
fn package_outputs(manifest: &Path) -> Vec<String> {
    use crate::resolution::Json;
    fn collect(value: &Json, out: &mut Vec<String>) {
        match value {
            Json::String(s) => out.push(s.clone()),
            Json::Array(items) => items.iter().for_each(|v| collect(v, out)),
            Json::Object(entries) => entries.iter().for_each(|(_, v)| collect(v, out)),
            _ => {}
        }
    }
    let Some(json) = read_small(manifest).and_then(|t| Json::parse(&t)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for field in [
        "main", "module", "types", "typings", "browser", "bin", "exports", "files",
    ] {
        if let Some(value) = json.get(field) {
            collect(value, &mut out);
        }
    }
    out
}

/// `compilerOptions.outDir` of a `tsconfig.json`, if it is a plain path.
fn tsconfig_out_dir(path: &Path) -> Option<String> {
    use crate::resolution::Json;
    let json = Json::parse_jsonc(&read_small(path)?)?;
    let out = json.get("compilerOptions")?.get("outDir")?.as_str()?;
    (!out.contains("${")).then(|| out.to_string())
}

/// A settings file's text, unless it is missing or implausibly large.
fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Recursively scans a repository and returns filtered file paths.
///
/// Rules:
/// - Traverses recursively from `root`
/// - Skips `node_modules`, `.git` (folder or worktree/submodule file),
///   and `build`, `dist`, `target` and `coverage` folders that hold
///   build output (see [`is_skipped_dir`])
/// - Honours `.gitignore`, `.ignore` and `.git/info/exclude` the way git
///   does, including nested ignore files and `!` re-includes, whether or not
///   the directory is a git checkout.  The user's global git excludes are not
///   read, so the same repository gives the same result on every machine.
/// - Hidden files are scanned (`.github/`, `.env.example` carry signal)
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

    let mut files: Vec<PathBuf> = WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry({
            let root = root.to_path_buf();
            move |entry| !is_ignored_dir(&root, entry)
        })
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
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
        for ignored in ALWAYS_IGNORED.iter().chain(OUTPUT_NAMES) {
            let d = root.join(ignored);
            fs::create_dir_all(&d).unwrap();
            File::create(d.join("ignored.txt")).unwrap();
        }

        let mut result = scan_repository(&root);

        // Normalize expected order
        result.sort_unstable();

        assert!(result.contains(&keep_file));
        assert!(result.contains(&keep_nested_file));

        for ignored in ALWAYS_IGNORED.iter().chain(OUTPUT_NAMES) {
            let ignored_file = root.join(ignored).join("ignored.txt");
            assert!(!result.contains(&ignored_file));
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn honours_gitignore_files() {
        let root = unique_temp_dir("codedna_scanner_gitignore_test");
        fs::write(root.join(".gitignore"), "generated/\n*.log\n!keep.log\n").unwrap();
        fs::create_dir_all(root.join("generated")).unwrap();
        fs::write(root.join("generated/out.js"), "x").unwrap();
        fs::write(root.join("debug.log"), "x").unwrap();
        fs::write(root.join("keep.log"), "x").unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("pkg/.gitignore"), "local.ts\n").unwrap();
        fs::write(root.join("pkg/local.ts"), "x").unwrap();
        fs::write(root.join("pkg/main.ts"), "x").unwrap();
        fs::create_dir_all(root.join(".github")).unwrap();
        fs::write(root.join(".github/ci.yml"), "x").unwrap();
        // A worktree or submodule has a `.git` file instead of a folder.
        fs::write(root.join(".git"), "gitdir: /elsewhere\n").unwrap();

        let result = scan_repository(&root);
        assert!(!result.contains(&root.join(".git")));

        assert!(!result.contains(&root.join("generated/out.js")));
        assert!(!result.contains(&root.join("debug.log")));
        assert!(!result.contains(&root.join("pkg/local.ts")));
        assert!(result.contains(&root.join("keep.log")));
        assert!(result.contains(&root.join("pkg/main.ts")));
        assert!(result.contains(&root.join(".github/ci.yml")));

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
