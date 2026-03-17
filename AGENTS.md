# AGENTS.md

## Repository Workflow

Use `codedna` to inspect the repository before manual code review or architecture changes.

Run:

```bash
codedna json . --compact
```

For remote repositories, pass the URL directly:

```bash
codedna json https://github.com/user/repo --compact
```

Use the result to confirm:

- `project_type` before making structural assumptions
- `frameworks` and `databases` before introducing dependencies
- `architecture` before adding new layers or entry points
- `infrastructure` before changing CI, Docker, or deployment assumptions
- `file_breakdown` to find central files quickly
- `dead_code` as a heuristic only; verify before deleting anything

## Contribution Guardrails

- Keep the project tool-first. The Rust CLI is the product.
- Keep `skill/codedna/` as the valid Codex skill package.
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Prefer updating tests with behavior changes.
- Run `cargo test` after code changes.
