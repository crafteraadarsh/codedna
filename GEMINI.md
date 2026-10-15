# GEMINI.md

## Project Context

Run:

```bash
codedna context .
```

If the CodeDna MCP tools are connected, call `get_context` instead. For full JSON use `codedna json . --compact`; for a remote repository pass its `https://` URL.

Use the result to confirm the project type, stack, architecture, entry points and key files before proposing changes.

## Guardrails

- Keep the Rust CLI as the primary product.
- `skill/codedna/SKILL.md` is the canonical skill; keep its copies identical (CI checks this).
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Treat dead-code output as a hint, not proof.
- Run `cargo test` after code changes.
