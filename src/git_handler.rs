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
/// Recognised prefixes: `http://`, `https://`, `git@`.  Plain `http://` is
/// recognised so it is never mistaken for a path, but `clone_repo` refuses it.
pub fn is_git_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://") || input.starts_with("git@")
}

/// Return `url` with any credentials (`user:token@`) replaced by `***`.
///
/// Use this whenever a URL the user typed is printed, so a token embedded in
/// it never reaches the terminal, logs or CI output.
pub fn redact_url(url: &str) -> String {
    match userinfo_range(url) {
        Some((start, end)) => format!("{}***{}", &url[..start], &url[end..]),
        None => url.to_string(),
    }
}

/// Byte range of the `user:password` part of a `scheme://user:password@host/...` URL.
fn userinfo_range(url: &str) -> Option<(usize, usize)> {
    let start = url.find("://")? + 3;
    let rest = &url[start..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let at = authority.rfind('@')?;
    if at == 0 {
        None
    } else {
        Some((start, start + at))
    }
}

/// Remove the credentials in `url` from `text` and make it safe to print.
fn scrub(text: &str, url: &str) -> String {
    let text = match userinfo_range(url) {
        Some((start, end)) => text
            .replace(url, &redact_url(url))
            .replace(&format!("{}@", &url[start..end]), "***@"),
        None => text.to_string(),
    };
    crate::safe_text::sanitize(&text)
}

// ── Repository cloning ───────────────────────────────────────────────────────

/// Clone a remote Git repository into a temporary directory.
///
/// Returns the `TempDir` handle.  The caller must keep this value alive for
/// the duration of the analysis — dropping it deletes the cloned files.
///
/// Uses a shallow fetch (depth 1) to minimise download size.
pub fn clone_repo(url: &str) -> Result<TempDir, String> {
    clone_repo_with_depth(url, 1)
}

/// Clone like [`clone_repo`], fetching the newest `depth` commits of history.
pub fn clone_repo_with_depth(url: &str, depth: i32) -> Result<TempDir, String> {
    if url.starts_with("http://") {
        return Err(format!(
            "Refusing to clone over unencrypted http://: {}. Use https:// instead.",
            scrub(url, url)
        ));
    }

    let temp_dir = TempDir::new().map_err(|e| format!("Failed to create temp directory: {e}"))?;

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.depth(depth.max(1));

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
    resolve_input_with_depth(input, 1)
}

/// Like [`resolve_input`], but a cloned repository keeps the newest `depth`
/// commits of history.
pub fn resolve_input_with_depth(
    input: &str,
    depth: i32,
) -> Result<(std::path::PathBuf, Option<TempDir>), String> {
    if is_git_url(input) {
        let temp_dir = clone_repo_with_depth(input, depth)?;
        let path = temp_dir.path().to_path_buf();
        Ok((path, Some(temp_dir)))
    } else {
        let path = Path::new(input).to_path_buf();
        if !path.exists() {
            let shown = crate::safe_text::sanitize(input);
            return Err(format!("Path not found: '{shown}'"));
        }
        Ok((path, None))
    }
}

// ── Error classification ─────────────────────────────────────────────────────

/// Translate raw `git2::Error` into a user-friendly message.
///
/// Credentials in `url` are removed from everything returned.
fn classify_clone_error(url: &str, err: &git2::Error) -> String {
    let shown_url = scrub(url, url);
    let raw = scrub(err.message(), url);
    let raw = raw.as_str();

    // Repository not found / access denied
    if raw.contains("not found")
        || raw.contains("404")
        || raw.contains("403")
        || raw.contains("authentication")
        || raw.contains("could not read Username")
    {
        return format!("Repository not found or access denied: {shown_url}");
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
        return format!("Invalid repository URL: {shown_url}");
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

    // ── redact_url ──────────────────────────────────────────────────────────

    #[test]
    fn redacts_token_in_https_url() {
        assert_eq!(
            redact_url("https://ghp_secret@github.com/user/repo"),
            "https://***@github.com/user/repo"
        );
        assert_eq!(
            redact_url("https://user:pass@github.com/user/repo.git"),
            "https://***@github.com/user/repo.git"
        );
    }

    #[test]
    fn leaves_urls_without_credentials_alone() {
        let url = "https://github.com/user/repo";
        assert_eq!(redact_url(url), url);
        assert_eq!(
            redact_url("git@github.com:user/repo.git"),
            "git@github.com:user/repo.git"
        );
        assert_eq!(redact_url("./local/path"), "./local/path");
    }

    #[test]
    fn redacts_short_credentials_without_touching_the_scheme() {
        assert_eq!(
            redact_url("https://h@example.com/r"),
            "https://***@example.com/r"
        );
    }

    #[test]
    fn ignores_at_sign_after_the_host() {
        let url = "https://github.com/user/repo/tree/main@v1";
        assert_eq!(redact_url(url), url);
    }

    #[test]
    fn scrub_removes_credentials_from_messages() {
        let url = "https://user:pass@example.com/repo";
        let msg = scrub("failed to fetch https://user:pass@example.com/repo\n", url);
        assert!(!msg.contains("pass"), "{msg}");
        assert!(!msg.contains('\n'), "{msg}");
    }

    #[test]
    fn refuses_plain_http_clone() {
        let err = clone_repo("http://user:pass@example.com/repo").unwrap_err();
        assert!(err.contains("https://"), "{err}");
        assert!(!err.contains("pass"), "{err}");
    }

    // ── clone_repo error paths ────────────────────────────────────────────────
    // These tests initialise the full libgit2 + vendored-OpenSSL stack.  On
    // Windows the OpenSSL atexit handler crashes (STATUS_ACCESS_VIOLATION)
    // after the test binary exits, so we skip them there.  The same code
    // paths are still exercised on Linux and macOS CI.

    #[test]
    #[cfg(not(target_os = "windows"))]
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
    #[cfg(not(target_os = "windows"))]
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
