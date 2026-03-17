//! Git handler module.
//!
//! Detects whether CLI input is a Git URL or a local path, clones remote
//! repositories into a temporary directory, and ensures cleanup after analysis.
//!
//! Uses `git2` for cloning and `tempfile` for temp directory management.

use std::path::Path;
use tempfile::TempDir;

// ── URL detection ─────────────────────────────────────────────────────────────

/// Returns `true` if `input` looks like a Git URL rather than a local path.
///
/// Recognised prefixes: `http://`, `https://`, `git@`.
pub fn is_git_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://") || input.starts_with("git@")
}

// ── Repository cloning ───────────────────────────────────────────────────────

/// Clone a remote Git repository into a temporary directory.
///
/// Returns the `TempDir` handle.  The caller must keep this value alive for
/// the duration of the analysis — dropping it deletes the cloned files.
///
/// Uses a shallow fetch (depth 1) to minimise download size.
pub fn clone_repo(url: &str) -> Result<TempDir, String> {
    let temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp directory: {e}"))?;

    // Set up fetch options for shallow clone (depth 1)
    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.depth(1);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder
        .clone(url, temp_dir.path())
        .map_err(|e| classify_clone_error(url, &e))?;

    Ok(temp_dir)
}

/// Resolve a CLI input to a local path, cloning first if it is a Git URL.
///
/// Returns `(Path, Option<TempDir>)`.  When the input was a URL the caller
/// **must** keep the `TempDir` alive until analysis is complete — dropping it
/// deletes the cloned repository.
pub fn resolve_input(input: &str) -> Result<(std::path::PathBuf, Option<TempDir>), String> {
    if is_git_url(input) {
        let temp_dir = clone_repo(input)?;
        let path = temp_dir.path().to_path_buf();
        Ok((path, Some(temp_dir)))
    } else {
        let path = Path::new(input).to_path_buf();
        if !path.exists() {
            return Err(format!("Path not found: '{input}'"));
        }
        Ok((path, None))
    }
}

// ── Error classification ─────────────────────────────────────────────────────

/// Translate raw `git2::Error` into a user-friendly message.
fn classify_clone_error(url: &str, err: &git2::Error) -> String {
    let raw = err.message();

    // Repository not found / access denied
    if raw.contains("not found")
        || raw.contains("404")
        || raw.contains("403")
        || raw.contains("authentication")
        || raw.contains("could not read Username")
    {
        return format!("Repository not found or access denied: {url}");
    }

    // Network connectivity issues
    if raw.contains("resolve")
        || raw.contains("connect")
        || raw.contains("timed out")
        || raw.contains("SSL")
        || raw.contains("network")
    {
        return format!("Network error while cloning: {raw}");
    }

    // Invalid URL
    if raw.contains("unsupported URL protocol") || raw.contains("invalid") {
        return format!("Invalid repository URL: {url}");
    }

    // Fallback
    format!("Failed to clone repository: {raw}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_git_url ────────────────────────────────────────────────────────────

    #[test]
    fn detects_https_url() {
        assert!(is_git_url("https://github.com/user/repo"));
    }

    #[test]
    fn detects_http_url() {
        assert!(is_git_url("http://github.com/user/repo"));
    }

    #[test]
    fn detects_git_ssh_url() {
        assert!(is_git_url("git@github.com:user/repo.git"));
    }

    #[test]
    fn rejects_local_dot_path() {
        assert!(!is_git_url("."));
    }

    #[test]
    fn rejects_local_relative_path() {
        assert!(!is_git_url("./src"));
    }

    #[test]
    fn rejects_local_absolute_path() {
        assert!(!is_git_url("/home/user/project"));
    }

    #[test]
    fn rejects_empty_string() {
        assert!(!is_git_url(""));
    }

    #[test]
    fn rejects_bare_word() {
        assert!(!is_git_url("myproject"));
    }

    // ── clone_repo error paths ────────────────────────────────────────────────

    #[test]
    fn clone_nonexistent_repo_returns_error() {
        let result = clone_repo("https://github.com/nonexistent-user-abc123/no-such-repo-xyz789");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        // Should be a user-friendly message, not raw git2 internals
        assert!(
            msg.contains("not found")
                || msg.contains("Failed to clone")
                || msg.contains("Network error"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn clone_invalid_url_returns_error() {
        let result = clone_repo("https://");
        assert!(result.is_err());
    }

    // ── resolve_input ─────────────────────────────────────────────────────────

    #[test]
    fn resolve_local_path_returns_none_tempdir() {
        let (path, temp) = resolve_input(".").unwrap();
        assert!(path.exists());
        assert!(temp.is_none());
    }

    #[test]
    fn resolve_missing_local_path_returns_error() {
        let result = resolve_input("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path not found"));
    }

    // ── temp dir lifecycle ────────────────────────────────────────────────────

    #[test]
    fn temp_dir_is_deleted_on_drop() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().to_path_buf();
        assert!(path.exists());
        drop(temp);
        assert!(!path.exists(), "temp dir should be deleted after drop");
    }

    // ── integration tests (require network) ──────────────────────────────────
    // Run with: cargo test -- --ignored

    /// Clone a known public repo and verify the temp dir contains files.
    #[test]
    #[ignore]
    fn clone_public_repo_succeeds() {
        let temp_dir = clone_repo("https://github.com/crafteraadarsh/codedna").unwrap();
        let path = temp_dir.path();

        // The cloned repo should contain at least a Cargo.toml
        assert!(
            path.join("Cargo.toml").exists(),
            "expected Cargo.toml in cloned repo"
        );
        // And a src directory
        assert!(
            path.join("src").is_dir(),
            "expected src/ directory in cloned repo"
        );
    }

    /// Verify temp dir is cleaned up after clone + drop.
    #[test]
    #[ignore]
    fn temp_dir_deleted_after_clone_and_drop() {
        let temp_dir = clone_repo("https://github.com/crafteraadarsh/codedna").unwrap();
        let cloned_path = temp_dir.path().to_path_buf();
        assert!(cloned_path.exists(), "cloned path should exist before drop");
        drop(temp_dir);
        assert!(
            !cloned_path.exists(),
            "cloned path should be deleted after drop"
        );
    }

    /// Full analyze pipeline via a Git URL — the core v1.1 integration test.
    #[test]
    #[ignore]
    fn full_analyze_with_git_url() {
        let url = "https://github.com/crafteraadarsh/codedna";
        let (local_path, _guard) = resolve_input(url).unwrap();

        // Run the full analysis pipeline
        let result = crate::analysis::analyze(&local_path);

        // Basic smoke checks: result should contain meaningful data
        assert!(result.total_loc > 0, "total_loc should be > 0");
        assert!(
            !result.languages.is_empty(),
            "languages should not be empty"
        );
        assert!(
            !result.file_breakdown.is_empty(),
            "file_breakdown should not be empty"
        );

        // Should detect Rust as a language (this IS a Rust project)
        assert!(
            result.languages.keys().any(|l| format!("{l}") == "Rust"),
            "expected Rust in languages"
        );

        // Guard is still alive — temp dir should exist
        assert!(
            local_path.exists(),
            "temp dir should exist while _guard is alive"
        );

        // Drop guard and verify cleanup
        let saved_path = local_path.clone();
        drop(_guard);
        assert!(
            !saved_path.exists(),
            "temp dir should be deleted after _guard is dropped"
        );
    }
}
