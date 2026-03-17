---
name: codedna
description: Analyze a repository with the `codedna` CLI before manual code spelunking. Use when the user asks for the tech stack, architecture, languages, framework usage, dead code, directory structure, LOC breakdown, or a quick high-level understanding of an unfamiliar codebase.
license: MIT
compatibility: opencode
metadata:
  repository: https://github.com/crafteraadarsh/codedna
---

# CodeDNA

Use `codedna` to build fast repository context before reading many files by hand.

## Quick Start

Run the narrowest command that answers the request:

- `codedna json . --compact` for agent consumption and structured summaries
- `codedna analyze .` for a human-readable overview
- `codedna stack .` for languages, frameworks, and databases
- `codedna files .` for per-file LOC hotspots
- `codedna framework <name> .` to locate framework usage
- `codedna deadcode .` to inspect unreachable files
- `codedna map . --depth 3` to inspect project layout

Prefer `codedna json` when you will reuse the result across several follow-up steps.

## Workflow

1. Start at the repository root unless the user names a subdirectory.
2. Run the least expensive command that fits the question.
3. Validate surprising claims by opening a few representative files instead of trusting heuristics blindly.
4. Cite `codedna` findings as generated analysis, not as guaranteed truth.

## Interpreting Output

- Treat framework, database, architecture, and dead-code results as heuristics.
- Use `codedna files` to identify large or central files before deeper review.
- Use `codedna framework <name>` to find concrete file references for follow-up inspection.
- When using `codedna json`, summarize the relevant fields instead of dumping raw JSON unless the user asked for it.

## Fallback

If `codedna` is unavailable in `PATH`, tell the user briefly and continue with normal repository inspection unless the user explicitly wants the tool installed or repaired.
