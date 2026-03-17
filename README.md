# CodeDna 🧬

> Rust-powered codebase intelligence engine and agent skill.

[![CI](https://github.com/crafteraadarsh/codedna/workflows/CI/badge.svg)](https://github.com/crafteraadarsh/codedna/actions)
[![Tests](https://img.shields.io/badge/tests-155%20passing-brightgreen)](#)
[![License](https://img.shields.io/badge/license-MIT-blue)](#license)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](#)

CodeDna analyzes any **local or remote** repository and reveals its **DNA** — the tech stack, architecture, LOC distribution, framework usage, infrastructure, and dead code — in milliseconds.

---

## What It Does

Given any repository (local path or Git URL), CodeDna produces a complete intelligence report:

| Area | Details |
|---|---|
| **Tech Stack** | Languages, frameworks, databases, tooling |
| **LOC Distribution** | Lines of code per language with visual bar chart |
| **Framework Usage** | Which files import which frameworks |
| **Architecture** | Frontend → API → Database pattern detection |
| **Infrastructure** | Docker, GitHub Actions, Kubernetes detection |
| **Dead Code** | Unused files unreachable from entry points |
| **Directory Map** | Tree-style repository structure |
| **JSON Output** | Structured output for AI agent consumption |

---

## Install

### From Source

```bash
git clone https://github.com/crafteraadarsh/codedna
cd codedna
cargo install --path .
```

### Via Cargo (once published)

```bash
cargo install codedna
```

Requires **Rust stable**. Install Rust at [rustup.rs](https://rustup.rs).

---

## Quick Start

### Local Repository

```bash
# Full intelligence report
codedna analyze .

# Machine-readable JSON (for AI agents)
codedna json .

# Tech stack only
codedna stack .

# Directory tree
codedna map . --depth 3
```

### Remote Git Repository (v1.1)

```bash
# Analyze any public GitHub repo
codedna analyze https://github.com/vercel/next.js

# Works with all commands
codedna stack https://github.com/tokio-rs/tokio
codedna json https://github.com/denoland/deno --compact
codedna deadcode git@github.com:user/repo.git

# SSH URLs work too
codedna analyze git@github.com:user/repo.git
```

CodeDna automatically detects Git URLs, performs a shallow clone into a temp directory, runs the full analysis, and cleans up — no manual `git clone` needed.

---

## Example Output

```
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
  React  +  Vite  +  Express

  Databases
  ─────────
  PostgreSQL,  Redis

  Infrastructure
  ──────────────
  Docker  •  GitHub Actions

  Architecture
  ────────────
  Frontend → API → Database

  Languages
  ─────────
  TypeScript        ████████████████████████░░░░   78%     12,450 LOC
  JavaScript        ████████░░░░░░░░░░░░░░░░░░░░   13%      2,100 LOC
  CSS               ███░░░░░░░░░░░░░░░░░░░░░░░░░    7%      1,100 LOC
  Markdown          █░░░░░░░░░░░░░░░░░░░░░░░░░░░    2%        130 LOC

  Top 5 Files by LOC
  ──────────────────
  1  src/server.ts                                       340 LOC   TypeScript
  2  src/controllers/user.ts                             290 LOC   TypeScript
  3  src/api/routes.ts                                   240 LOC   TypeScript
  4  src/components/Dashboard.tsx                        180 LOC   TypeScript
  5  src/utils/auth.ts                                   160 LOC   TypeScript

  Dead Code
  ─────────
  src/utils/oldHelper.ts
  src/components/LegacyNavbar.tsx

  Summary
  ───────
  Total LOC          15,780
  Languages          4
  Frameworks         3
  Databases          2
  Infrastructure     2
  Files scanned      148
  Dead files         2
  Dependency links   87
```

---

## Commands

All commands accept either a **local path** or a **Git URL**.

| Command | Description |
|---|---|
| `codedna analyze [path or url]` | Full intelligence report |
| `codedna analyze [path or url] --time` | Report + elapsed time |
| `codedna stack [path or url]` | Languages, frameworks, databases |
| `codedna files [path or url]` | Per-file LOC breakdown |
| `codedna framework <name> [path or url]` | Files that import the given framework |
| `codedna deadcode [path or url]` | Unused / unreachable files |
| `codedna map [path or url]` | Directory tree |
| `codedna map [path or url] --depth N` | Directory tree limited to N levels |
| `codedna scan [path or url]` | Raw scanned file list |
| `codedna json [path or url]` | Full analysis as JSON |
| `codedna json [path or url] --compact` | Single-line JSON |
| `codedna json [path or url] --time` | JSON + elapsed time |

### Supported URL Formats

Any input starting with `http://`, `https://`, or `git@` is treated as a Git URL. Everything else is treated as a local path.

```
https://github.com/user/repo
https://gitlab.com/user/repo
http://github.com/user/repo
git@github.com:user/repo.git
git@gitlab.com:user/repo.git
```

### Framework Detection

The `framework` command supports per-file import scanning for:

**JavaScript / TypeScript:** React, Next.js, Express, Vite, Vue, Svelte, NestJS, Fastify, Koa, Remix, Gatsby, Nuxt, Astro

**Python:** FastAPI, Django, Flask, Starlette, aiohttp

**Rust:** Tokio, Axum, Actix-web, Rocket, Warp, Leptos, Dioxus, Yew, Tauri

**Go:** Gin, Echo, Fiber, Chi

```bash
codedna framework react .
codedna framework fastapi ./backend
codedna framework axum https://github.com/user/rust-project
```

---

## JSON Output

`codedna json .` outputs structured JSON for AI agent consumption:

```json
{
  "project_type": "Full-stack web application",
  "total_loc": 15780,
  "languages": {
    "TypeScript": 12450,
    "JavaScript": 2100,
    "CSS": 1100
  },
  "frameworks": ["React", "Vite", "Express"],
  "databases": ["PostgreSQL", "Redis"],
  "infrastructure": ["Docker", "GitHub Actions"],
  "architecture": "Frontend → API → Database",
  "dead_code": [
    "src/utils/oldHelper.ts",
    "src/components/LegacyNavbar.tsx"
  ],
  "dependency_graph": {
    "src/server.ts": ["src/api/routes.ts"],
    "src/api/routes.ts": ["src/controllers/user.ts"]
  },
  "file_breakdown": [
    { "file": "src/server.ts", "loc": 340, "language": "TypeScript" }
  ]
}
```

### Useful jq Recipes

```bash
# Project type and architecture
codedna json . | jq '{project_type, architecture}'

# All frameworks
codedna json . | jq '.frameworks[]'

# Languages sorted by LOC
codedna json . | jq '.languages | to_entries | sort_by(-.value)'

# Dead code files
codedna json . | jq '.dead_code[]'

# Top 5 files
codedna json . | jq '.file_breakdown[:5]'

# Compact agent summary
codedna json . --compact | jq -c '{project_type,frameworks,databases,architecture,total_loc}'
```

---

## How It Works

```
Input (local path or Git URL)
    │
    ▼
Git Handler (git2)             — detect URL vs path, shallow-clone if remote
    │
    ▼
Scanner (walkdir)              — recursive walk, ignore node_modules/.git/
    │
    ├──► Language Detector     — file extension → Language enum
    ├──► LOC Counter           — non-empty lines, skip binary/UTF-8 errors  (parallel)
    ├──► Framework Detector    — parse package.json / requirements.txt / Cargo.toml / go.mod
    ├──► Infra Detector        — detect Dockerfile / docker-compose / .github/workflows
    ├──► Dependency Graph      — parse import/require/mod statements         (parallel)
    └──► Dead Code Detector    — BFS from entry points over dependency graph
                │
                ▼
          AnalysisResult
                │
          ┌─────┴─────┐
          ▼           ▼
    CLI Report     JSON Output
                      │
                      ▼
               Cleanup (delete temp dir if remote)
```

All file-level steps run in parallel via `rayon`.

---

## Detection Coverage

### Languages

`.rs` `.ts` `.tsx` `.js` `.jsx` `.mjs` `.py` `.go` `.sol` `.css` `.scss` `.sass` `.html` `.htm` `.toml` `.json` `.yml` `.yaml` `.md` `.sh` `.bash` `.zsh`

### Frameworks

| Ecosystem | Frameworks |
|---|---|
| JS / TS | React, Next.js, Vue, Nuxt, Svelte, Astro, Remix, Gatsby, Express, Vite, NestJS, Fastify, Koa, Hapi |
| Python | FastAPI, Django, Flask, Starlette, Tornado, aiohttp, Pyramid, Sanic, Litestar |
| Rust | Tokio, Axum, Actix-web, Rocket, Warp, Tonic, Poem, Hyper, Tauri, Leptos, Dioxus, Yew |
| Go | Gin, Echo, Fiber, Gorilla Mux, Beego, Chi, Revel, Uber FX |

### Databases

| Source | Databases |
|---|---|
| `package.json` | PostgreSQL, MongoDB, MySQL, Redis, Prisma, SQLite, Sequelize, TypeORM, Drizzle ORM, Knex, Cassandra, Elasticsearch, DynamoDB |
| `requirements.txt` | SQLAlchemy, PostgreSQL, MongoDB, Redis, MySQL, Tortoise ORM, Peewee |
| `Cargo.toml` | SQLx, Diesel, SeaORM, PostgreSQL, MongoDB, Redis, SQLite, Sled, Elasticsearch |

### Infrastructure

| Signal | Technology |
|---|---|
| `Dockerfile` exists | Docker |
| `docker-compose.yml` / `.yaml` exists | Docker Compose |
| Files inside `.github/workflows/` | GitHub Actions |
| `.yaml` files in `k8s/` or `kubernetes/` | Kubernetes |
| `Makefile` exists | Makefile |

### Ignored Directories

`node_modules/` `.git/` `target/` `dist/` `build/` `coverage/`

---

## Performance

| Metric | Value |
|---|---|
| Local analysis (26 files, debug build) | ~2.6 ms |
| Remote clone | Shallow (`--depth 1`) — only latest commit |
| Parallelism | `rayon` thread pool — all CPU cores |
| Target for 100 000+ file repos | < 5 seconds |
| Memory model | Streaming per-file reads, no full-repo buffer |
| Temp dir cleanup | Automatic — deleted immediately after analysis |

Build in release mode for maximum performance:

```bash
cargo build --release
./target/release/codedna analyze .
```

---

## New in v1.1: Git Repository Support

v1.1 introduces remote Git repository analysis. CodeDna can now accept Git URLs directly — no manual cloning required.

### What Changed

| Area | Change |
|---|---|
| **New module** | `git_handler.rs` — URL detection, shallow clone, temp dir management |
| **CLI arguments** | All `[path]` args now accept `[path or url]` |
| **Dependencies** | Added `git2` (with vendored OpenSSL) and `tempfile` crates |
| **Path normalization** | Output paths are identical whether source is local or remote |
| **Error handling** | User-friendly messages for bad URLs, network failures, auth errors |
| **Cross-platform** | Tested on Linux, macOS, and Windows via CI |

### Breaking Changes

**None.** v1.1 is fully backward compatible with v1.0. All local path usage works exactly as before.

---

## AI Agent Integration

CodeDna is designed to be the **first thing any AI agent runs on a new codebase**.

```bash
# Give your agent instant context for a local project
codedna json . --compact | jq -c \
  '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'

# Or analyze any public repo directly
codedna json https://github.com/user/repo --compact | jq -c \
  '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'
```

The full skill specification is in [`SKILL.md`](SKILL.md).

---

### Supported Agents

| Agent | Config file | Status |
|---|---|---|
| Claude Code | `CLAUDE.md` | Supported |
| OpenCode | `AGENTS.md` | Supported |
| Codex CLI | `AGENTS.md` | Supported |
| Cursor | `.cursorrules` | Supported |
| Gemini CLI | `GEMINI.md` | Supported |

---

### Claude Code

Create a `CLAUDE.md` in your project root:

```markdown
## Before starting any task

Run this first and read the output before writing any code:

```bash
codedna json . --compact
```

Use the output to understand:
- `project_type` — what kind of project this is
- `frameworks` — frameworks already in use (do not add duplicates)
- `databases` — databases connected (match existing patterns)
- `architecture` — how the layers communicate
- `dead_code` — files to avoid or safely delete
```

Or add to your **global** Claude Code config (`~/.claude/CLAUDE.md`) so it applies to every project automatically.

---

### OpenCode

Create an `AGENTS.md` in your project root:

```markdown
## Codebase Analysis — run first on every task

Before starting any task, run:

```bash
codedna json . --compact
```

Use `project_type`, `frameworks`, `databases`, `architecture`, and `dead_code`
to understand the codebase before generating any code.
```

OpenCode reads `AGENTS.md` automatically at the start of every session.

---

### Codex CLI

Same `AGENTS.md` format as OpenCode, or inject context inline:

```bash
# Inject CodeDna context into a Codex CLI prompt
CONTEXT=$(codedna json . --compact)
codex "Codebase context: $CONTEXT. Now add a user authentication endpoint."
```

---

### Cursor

Add to your `.cursorrules` file in the project root:

```
## Project Analysis
Before suggesting any code, run `codedna json . --compact` in the terminal
and use the output to understand the project stack, architecture, and dead code.

## Rules derived from CodeDna
- Match detected frameworks — do not introduce new ones without asking
- Follow the detected architecture pattern
- Do not import from files listed in dead_code
- Match the dominant language (highest LOC %) for new files
```

---

### Gemini CLI

Create a `GEMINI.md` in your project root:

```markdown
## Project context

Run the following before starting any task:

```bash
codedna json . --compact
```

Use `project_type`, `frameworks`, `databases`, `architecture`, and `dead_code`
to understand the codebase before generating code.
```

---

### Any Agent — Universal One-Liner

```bash
# macOS — copy context to clipboard
codedna json . --compact | pbcopy

# Linux — copy context to clipboard
codedna json . --compact | xclip -selection clipboard

# Analyze a remote repo without cloning it yourself
codedna analyze https://github.com/vercel/next.js

# Or just print a human-readable summary
codedna analyze .
```

Paste into your agent's chat as the first message before any task.

---

### Useful jq Recipes

```bash
# Project type and architecture
codedna json . | jq '{project_type, architecture}'

# All frameworks
codedna json . | jq '.frameworks[]'

# Languages sorted by LOC
codedna json . | jq '.languages | to_entries | sort_by(-.value)'

# Dead code files
codedna json . | jq '.dead_code[]'

# Top 5 files by LOC
codedna json . | jq '.file_breakdown[:5]'

# Check if Docker is used
codedna json . | jq '.infrastructure | contains(["Docker"])'

# Minimal one-line agent context
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'
```

---

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

```bash
# Quick contribution loop
cargo test          # must pass
cargo clippy        # must be clean
cargo fmt           # apply formatting
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide on adding new frameworks,
languages, databases, and infrastructure detection rules.

---

## License

MIT — see [LICENSE](LICENSE).

---

<div align="center">
  <strong>CodeDna</strong> — codebase intelligence for humans and AI agents 🧬
</div>
