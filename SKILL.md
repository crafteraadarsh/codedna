# CodeDna Skill

```yaml
name: codedna-analyzer
version: 1.1.0
description: >
  Analyze any local or remote repository and reveal its codebase DNA —
  tech stack, architecture, LOC distribution, framework usage,
  infrastructure, and dead code.
tools:
  - codedna
repository: https://github.com/crafteraadarsh/codedna
```

---

## Quick Start

```bash
# Install
cargo install --git https://github.com/crafteraadarsh/codedna

# Analyze a local repo
codedna analyze .

# Analyze a remote repo (v1.1)
codedna analyze https://github.com/vercel/next.js

# JSON output for agents
codedna json . --compact
codedna json https://github.com/user/repo --compact
```

---

## Commands

All commands accept either a **local path** or a **Git URL**.

| Command | Description |
|---|---|
| `codedna analyze [path or url]` | Full intelligence report |
| `codedna stack [path or url]` | Languages, frameworks, databases |
| `codedna files [path or url]` | Per-file LOC breakdown |
| `codedna framework <name> [path or url]` | Files importing a framework |
| `codedna deadcode [path or url]` | Unreachable files |
| `codedna map [path or url]` | Directory tree |
| `codedna json [path or url]` | Full analysis as JSON |

---

## Agent Integration

Agent-specific config files are provided at the repo root:

| Agent | Config File |
|---|---|
| OpenCode / Codex CLI | [`AGENTS.md`](AGENTS.md) |
| Claude Code | [`CLAUDE.md`](CLAUDE.md) |
| Gemini CLI | [`GEMINI.md`](GEMINI.md) |
| Cursor | [`.cursorrules`](.cursorrules) |

Each instructs the agent to run `codedna json . --compact` before any task.

---

## Full Skill Specification

For the complete skill definition including JSON schema, detection coverage, architecture inference, jq recipes, and examples:

**[skills/codedna-analyzer/SKILL.md](skills/codedna-analyzer/SKILL.md)**

---

## Links

- **Repository:** https://github.com/crafteraadarsh/codedna
- **Install:** `cargo install --git https://github.com/crafteraadarsh/codedna`
- **Issues:** https://github.com/crafteraadarsh/codedna/issues
