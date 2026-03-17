# Example: Remote Git Repository Analysis

Analyze any public Git repository without cloning it manually.

## Analyze a GitHub Repository

```bash
$ codedna analyze https://github.com/vercel/next.js

╔════════════════════════════════════════╗
║                                        ║
║      CodeDna Intelligence Report      ║
║                                        ║
╚════════════════════════════════════════╝

  Project Type
  ────────────
  Full-stack web application

  Stack
  ─────
  React  +  Next.js  +  TypeScript

  ...
```

CodeDna automatically detects the Git URL, performs a shallow clone (`--depth 1`) into a temporary directory, runs the full analysis pipeline, and deletes the temp directory when done.

## Supported URL Formats

```bash
# HTTPS (most common)
codedna analyze https://github.com/vercel/next.js
codedna analyze https://gitlab.com/user/repo

# HTTP
codedna analyze http://github.com/user/repo

# SSH
codedna analyze git@github.com:user/repo.git
codedna analyze git@gitlab.com:user/repo.git
```

## JSON Output for Remote Repos

```bash
# Get compact JSON for an agent to consume
$ codedna json https://github.com/tokio-rs/tokio --compact | jq -c '{project_type,frameworks,total_loc}'
{"project_type":"Rust library / CLI tool","frameworks":["Tokio"],"total_loc":245000}
```

## All Commands Work with URLs

```bash
# Tech stack of a remote repo
codedna stack https://github.com/denoland/deno

# Dead code in a remote repo
codedna deadcode https://github.com/user/project

# Directory map of a remote repo
codedna map https://github.com/user/project --depth 2

# Per-file LOC breakdown
codedna files https://github.com/user/project
```

## Error Handling

```bash
# Non-existent repository
$ codedna analyze https://github.com/user/nonexistent-repo
Error: Repository not found — check the URL and ensure it exists

# Network failure
$ codedna analyze https://github.com/user/repo
Error: Network error — check your internet connection

# Invalid URL
$ codedna analyze https://not-a-git-host/
Error: Failed to clone repository — verify the URL is a valid Git repository
```

## How It Works

1. CodeDna detects that the input starts with `https://`, `http://`, or `git@`
2. Performs a shallow clone (`--depth 1`) into a temp directory via `git2`
3. Runs the standard analysis pipeline on the cloned content
4. Returns results with normalized file paths (identical format to local analysis)
5. Deletes the temp directory automatically
