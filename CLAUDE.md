# CLAUDE.md

## Before Starting Work

Run:

```bash
codedna context .
```

If the CodeDna MCP tools are connected, call `get_context` instead. For full JSON use `codedna json . --compact`; for a remote repository pass its `https://` URL.

Use the output to understand:

- the project type before making structural assumptions
- frameworks before adding libraries or patterns
- databases before changing persistence code
- infrastructure before changing CI, Docker or deployment assumptions
- architecture and entry points before introducing new layers
- dead-code candidates as a heuristic only; verify before deleting anything

## Repository Rules

- Keep CodeDna tool-first; the Rust CLI is the product.
- `skill/codedna/SKILL.md` is the canonical skill; keep its copies identical (CI checks this).
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Match existing naming and module patterns in `src/`.
- Update tests when behavior changes.
- Run `cargo test` after changes.
