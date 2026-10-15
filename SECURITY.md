# Security Policy

## What CodeDna does with your code

CodeDna is built to be safe to run on any repository, including ones you do not trust.

- **Nothing leaves your machine.** There is no telemetry, analytics or update check. The only network access is cloning a Git URL you pass on the command line.
- **Repository code is never run.** CodeDna only reads files. It does not execute scripts, build steps, hooks or package managers from the scanned repository.
- **It stays inside the repository.** Symbolic links are not followed, so a repository cannot point the scan at files elsewhere on your machine.
- **Credentials are never printed.** If a clone URL contains a token (`https://user:token@host/...`), it is replaced with `***` in all output and error messages.
- **Clones are encrypted.** Plain `http://` URLs are refused; use `https://` or SSH.
- **Clones are temporary.** Remote repositories are shallow-cloned into a temporary directory that is deleted when the command finishes.
- **The MCP server is read-only and fenced.** `codedna mcp` only reads folders inside the directory it was started in, or ones you allow with `--root`; `..` and symbolic links cannot escape them. Remote repositories must be `https://` URLs. Tools return analysis results, never file contents, and every response is size-capped.
- **Hostile files are handled.** Files over 10 MiB are skipped, and file names containing terminal control or text-direction characters are never printed as-is.

Dependencies are checked against the RustSec advisory database on every pull request.

## Verifying a download

Release binaries are built by GitHub Actions from the tagged source. Each archive ships with a SHA-256 checksum (the install scripts check it before installing) and a signed build-provenance attestation:

```bash
gh attestation verify <archive> --repo crafteraadarsh/codedna
```

## Supported versions

Security fixes go into the latest release.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue.

Include what you found, how to reproduce it, and the impact you expect. You will get a reply within 7 days, and a fix or a plan once the report is confirmed. Reporters are credited in the release notes unless they prefer otherwise.
