# CLAUDE.md

## Before Starting Work

Run:

```bash
codedna json . --compact
```

For remote repositories:

```bash
codedna json https://github.com/user/repo --compact
```

Use the output to understand:

- `project_type` before making structural assumptions
- `frameworks` before adding libraries or patterns
- `databases` before changing persistence code
- `infrastructure` before changing CI, Docker, or deployment assumptions
- `architecture` before introducing new layers
- `dead_code` as a heuristic only; verify before deleting anything

## Repository Rules

- Keep CodeDna tool-first; the Rust CLI is the product.
- Keep `skill/codedna/` as the valid Codex skill package.
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Match existing naming and module patterns in `src/`.
- Update tests when behavior changes.
- Run `cargo test` after changes.
