# AGENTS.md

## Repository Workflow

Use `codedna` to understand the repository before manual code review or architecture changes.

Start with:

```bash
codedna context .
```

It prints a short briefing: project type, stack, architecture, entry points, key files and a directory map. If the CodeDna MCP tools are connected, call `get_context` instead.

For full JSON, or a remote repository:

```bash
codedna json . --compact
codedna context https://github.com/user/repo
```

Use the result to confirm:

- the project type before making structural assumptions
- frameworks and databases before introducing dependencies
- architecture and entry points before adding new layers
- infrastructure before changing CI, Docker or deployment assumptions
- dead-code candidates as a heuristic only; verify before deleting anything

## Contribution Guardrails

- Keep the project tool-first. The Rust CLI is the product.
- `skill/codedna/SKILL.md` is the canonical skill. Copies live in `.agents/skills/`, `.claude/skills/`, `.opencode/skills/` and `plugins/codedna/skills/`; keep them identical (CI checks this).
- Keep `skills/codedna-analyzer/` as the detailed cross-agent reference docs.
- Update tests with behavior changes.
- Run `cargo test` after code changes.
