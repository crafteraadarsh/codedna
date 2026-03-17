# GEMINI.md

## Project Context

Run:

```bash
codedna json . --compact
```

For remote repositories:

```bash
codedna json https://github.com/user/repo --compact
```

Use the result to confirm the project type, frameworks, databases, architecture, and major files before proposing changes.

## Guardrails

- Keep the Rust CLI as the primary product.
- Keep `skill/codedna/` as the valid Codex skill package.
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Treat dead-code output as a hint, not proof.
- Run `cargo test` after code changes.
