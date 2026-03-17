# CodeDna 🧬

**Codebase intelligence engine and AI agent skill.**

[![CI](https://github.com/crafteraadarsh/codedna/workflows/CI/badge.svg)](https://github.com/crafteraadarsh/codedna/actions)
[![Tests](https://img.shields.io/badge/tests-155%20passing-brightgreen)](#)
[![License](https://img.shields.io/badge/license-MIT-blue)](#license)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](#)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey)](#)

Point CodeDna at any local directory or remote Git URL. It scans the repository and produces a complete intelligence report — tech stack, architecture, LOC distribution, framework usage, infrastructure, dead code — in milliseconds. Output is both human-readable and machine-readable (JSON), making it the ideal first command for any AI coding agent.

```bash
codedna analyze .
codedna analyze https://github.com/vercel/next.js
codedna json . --compact
```

This repository is primarily a **Rust CLI tool**. It also ships agent integration files so CodeDna can be used cleanly from **Codex, OpenCode, and Claude**.

---

## Table of Contents

- [Install](#install)
- [Quick Start](#quick-start)
- [Commands](#commands)
- [Example Output](#example-output)
- [JSON Output](#json-output)
- [How It Works](#how-it-works)
- [Git URL Support](#git-url-support)
- [Detection Coverage](#detection-coverage)
- [Performance](#performance)
- [AI Agent Skill](#ai-agent-skill)
- [Project Structure](#project-structure)
- [Contributing](#contributing)
- [License](#license)

---

## Install CLI

### From GitHub

```bash
cargo install --git https://github.com/crafteraadarsh/codedna
```

### From Source

```bash
git clone https://github.com/crafteraadarsh/codedna
cd codedna
cargo install --path .
```

Requires **Rust stable**. Install Rust at [rustup.rs](https://rustup.rs).

Verify installation:

```bash
codedna --version
```
---

## Agent Support

CodeDna integrates with agents in two different ways:

- `Codex`: install the packaged skill in `skill/codedna/`
- `OpenCode`: add an `AGENTS.md` bootstrap file to the target repository
- `Claude`: add a `CLAUDE.md` bootstrap file to the target repository

## Install Codex Skill

If you want Codex to use CodeDna as a real skill, use the dedicated package in `skill/codedna/`.

- `skill/codedna/SKILL.md` contains the trigger description and workflow
- `skill/codedna/agents/openai.yaml` contains Codex UI metadata

Install it with:

```bash
./scripts/install-codex-skill.sh
```

Package it as a distributable tarball with:

```bash
./scripts/package-codex-skill.sh
```

## Bootstrap OpenCode And Claude

To copy portable bootstrap files into another repository:

```bash
./scripts/bootstrap-agent-files.sh /path/to/target-repo opencode claude
```

To install the Codex skill and bootstrap OpenCode plus Claude in one go:

```bash
./scripts/bootstrap-agent-files.sh /path/to/target-repo codex opencode claude
```

---

## Quick Start

```bash
# Full intelligence report (local)
codedna analyze .

# Full intelligence report (remote — auto-clones)
codedna analyze https://github.com/vercel/next.js

# JSON for AI agents
codedna json . --compact

# Tech stack only
codedna stack .

# Dead code detection
codedna deadcode https://github.com/user/repo

# Directory tree
codedna map . --depth 3

# Find files using a specific framework
codedna framework react .
```

---

## Commands

All commands accept a **local path** or a **Git URL**.

| Command | Description |
|---|---|
| `codedna analyze [path\|url]` | Full intelligence report |
| `codedna analyze [path\|url] --time` | Report with elapsed time |
| `codedna stack [path\|url]` | Languages, frameworks, databases |
| `codedna files [path\|url]` | Per-file LOC breakdown |
| `codedna framework <name> [path\|url]` | Files importing a given framework |
| `codedna deadcode [path\|url]` | Unreachable / unused files |
| `codedna map [path\|url]` | Directory tree |
| `codedna map [path\|url] --depth N` | Directory tree limited to N levels |
| `codedna scan [path\|url]` | Raw scanned file list |
| `codedna json [path\|url]` | Full analysis as JSON |
| `codedna json [path\|url] --compact` | Single-line JSON |
| `codedna json [path\|url] --time` | JSON with elapsed time |

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

## JSON Output

`codedna json .` produces structured JSON for programmatic consumption:

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

### JSON Fields

| Field | Type | Description |
|---|---|---|
| `project_type` | string | High-level project classification |
| `total_loc` | number | Total non-empty lines of code |
| `languages` | object | Language → LOC map |
| `frameworks` | string[] | Detected frameworks |
| `databases` | string[] | Detected databases / ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Inferred architecture pattern |
| `dead_code` | string[] | Unreachable file paths |
| `dependency_graph` | object | File → imported files map |
| `file_breakdown` | object[] | Per-file `{ file, loc, language }` |

### jq Recipes

```bash
# Compact agent context
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'

# All frameworks
codedna json . | jq '.frameworks[]'

# Languages sorted by LOC
codedna json . | jq '.languages | to_entries | sort_by(-.value)'

# Dead code files
codedna json . | jq '.dead_code[]'

# Top 5 largest files
codedna json . | jq '.file_breakdown[:5]'

# Check if Docker is used
codedna json . | jq '.infrastructure | contains(["Docker"])'
```

---

## How It Works

```
Input (local path or Git URL)
    │
    ▼
Git Handler                    — detect URL vs path; shallow-clone if remote
    │
    ▼
Scanner                        — recursive file walk, skip ignored directories
    │
    ├──► Language Detector     — file extension → language classification
    ├──► LOC Counter           — count non-empty lines per file            (parallel)
    ├──► Framework Detector    — parse manifests + per-file import scanning
    ├──► Infra Detector        — detect Docker, CI, Kubernetes, Makefile
    ├──► Dependency Graph      — parse import/require/mod statements       (parallel)
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

All file-level analysis runs in parallel via **rayon**.

---

## Git URL Support

CodeDna accepts Git URLs directly. No manual cloning required.

### Supported Formats

```
https://github.com/user/repo
https://gitlab.com/user/repo
http://github.com/user/repo
git@github.com:user/repo.git
git@gitlab.com:user/repo.git
```

Any input starting with `http://`, `https://`, or `git@` is treated as a Git URL. Everything else is a local path.

### What Happens

1. URL detected → shallow clone (`--depth 1`) into a temp directory
2. Standard analysis pipeline runs on the cloned content
3. Output paths are normalized (identical format to local analysis)
4. Temp directory is deleted automatically

### Error Handling

- **Repository not found** — clear message for non-existent repos
- **Network failure** — connectivity error with retry suggestion
- **Invalid URL** — message indicating the URL is not a valid Git repository
- **Auth required** — message for private repos needing authentication

---

## Detection Coverage

### Languages (22)

`.rs` `.ts` `.tsx` `.js` `.jsx` `.mjs` `.py` `.go` `.sol` `.css` `.scss` `.sass` `.html` `.htm` `.toml` `.json` `.yml` `.yaml` `.md` `.sh` `.bash` `.zsh`

### Frameworks (46)

| Ecosystem | Frameworks |
|---|---|
| **JS / TS** | React, Next.js, Vue, Nuxt, Svelte, Astro, Remix, Gatsby, Express, Vite, NestJS, Fastify, Koa, Hapi |
| **Python** | FastAPI, Django, Flask, Starlette, Tornado, aiohttp, Pyramid, Sanic, Litestar |
| **Rust** | Tokio, Axum, Actix-web, Rocket, Warp, Tonic, Poem, Hyper, Tauri, Leptos, Dioxus, Yew |
| **Go** | Gin, Echo, Fiber, Gorilla Mux, Beego, Chi, Revel, Uber FX |

### Databases (26)

| Source | Databases |
|---|---|
| **`package.json`** | PostgreSQL, MongoDB, MySQL, Redis, Prisma, SQLite, Sequelize, TypeORM, Drizzle ORM, Knex, Cassandra, Elasticsearch, DynamoDB |
| **`requirements.txt`** | SQLAlchemy, PostgreSQL, MongoDB, Redis, MySQL, Tortoise ORM, Peewee |
| **`Cargo.toml`** | SQLx, Diesel, SeaORM, PostgreSQL, MongoDB, Redis, SQLite, Sled, Elasticsearch |

### Infrastructure

| Signal | Technology |
|---|---|
| `Dockerfile` | Docker |
| `docker-compose.yml` / `.yaml` | Docker Compose |
| `.github/workflows/*.yml` | GitHub Actions |
| `k8s/` or `kubernetes/` YAML files | Kubernetes |
| `Makefile` | Makefile |

### Architecture Inference

| Signals | Result |
|---|---|
| Frontend + Backend + Database | `Frontend → API → Database` |
| Frontend + Backend | `Frontend → API` |
| Backend + Database | `API → Database` |
| Frontend only | `Frontend only` |
| Backend only | `API only` |
| None detected | `Monolithic / undetermined` |

### Ignored Directories

`node_modules/` `.git/` `target/` `dist/` `build/` `coverage/`

---

## Performance

| Metric | Value |
|---|---|
| Local analysis (26 files) | ~2.6 ms |
| Remote repos | Shallow clone (`--depth 1`) |
| Parallelism | rayon — all CPU cores |
| 100k+ file repos | < 5 seconds target |
| Memory | Streaming per-file reads |
| Temp dir cleanup | Automatic after analysis |

```bash
# Build in release mode for best performance
cargo build --release
./target/release/codedna analyze .
```

---

## AI Agent Skill

CodeDna is designed as the **first command any AI agent runs on a new codebase**.

### Skill Architecture

```
codedna/
├── skills/
│   └── codedna-analyzer/
│       ├── SKILL.md              ← full portable skill specification
│       └── examples/
│           ├── local-analysis.md
│           └── remote-analysis.md
├── skill/
│   └── codedna/
│       ├── SKILL.md              ← valid Codex skill package
│       └── agents/openai.yaml    ← Codex skill metadata
├── templates/
│   ├── AGENTS.md                 ← portable OpenCode template
│   └── CLAUDE.md                 ← portable Claude template
├── scripts/
│   ├── install-codex-skill.sh
│   ├── package-codex-skill.sh
│   └── bootstrap-agent-files.sh
├── SKILL.md                      ← quick reference + pointer doc
├── AGENTS.md                     ← OpenCode / Codex CLI
├── CLAUDE.md                     ← Claude Code
├── GEMINI.md                     ← Gemini CLI
└── .cursorrules                  ← Cursor
```

The detailed, portable reference lives in [`skills/codedna-analyzer/SKILL.md`](skills/codedna-analyzer/SKILL.md). The valid Codex skill package lives in [`skill/codedna/`](skill/codedna/).

### Agent Files In This Repo

| File | Purpose |
|---|---|
| `AGENTS.md` | Instructions for OpenCode and Codex-style repo guidance here |
| `CLAUDE.md` | Claude Code bootstrap for this repository |
| `GEMINI.md` | Gemini CLI bootstrap for this repository |
| `.cursorrules` | Cursor guidance for this repository |
| `skill/codedna/SKILL.md` | Codex skill trigger and workflow |
| `skill/codedna/agents/openai.yaml` | Codex UI metadata for the skill |
| `templates/AGENTS.md` | Portable OpenCode bootstrap template |
| `templates/CLAUDE.md` | Portable Claude bootstrap template |
| `scripts/bootstrap-agent-files.sh` | Copies templates into a target repository |

Each config file instructs the agent to run `codedna json . --compact` before any task and use the output to understand the project.

### One-Liner for Any Agent

```bash
# Get compact context
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'

# Or for a remote repo
codedna json https://github.com/user/repo --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'

# Copy to clipboard (macOS)
codedna json . --compact | pbcopy

# Copy to clipboard (Linux)
codedna json . --compact | xclip -selection clipboard
```

### Full Skill Specification

See [`skills/codedna-analyzer/SKILL.md`](skills/codedna-analyzer/SKILL.md) for the complete skill definition — JSON schema, field reference, detection tables, architecture inference, jq recipes, and annotated examples.

---

## Project Structure

```
src/
├── main.rs                 — entry point, module declarations
├── cli.rs                  — clap CLI definitions and command dispatch
├── git_handler.rs          — Git URL detection, shallow clone, temp dir management
├── analysis.rs             — analysis pipeline orchestration
├── scanner.rs              — recursive file walker (walkdir)
├── language_detector.rs    — file extension → language mapping
├── loc_counter.rs          — line counting with binary detection
├── framework_detector.rs   — manifest parsing + per-file import scanning
├── dependency_graph.rs     — import/require/mod statement parsing
├── dead_code_detector.rs   — BFS reachability from entry points
├── reporter.rs             — CLI report + JSON output formatting
└── repo_map.rs             — tree-style directory rendering
```

### Dependencies

| Crate | Purpose |
|---|---|
| `clap` 4 | CLI argument parsing (derive) |
| `walkdir` 2 | Recursive directory traversal |
| `serde` + `serde_json` | JSON serialization |
| `rayon` | Parallel file processing |
| `git2` 0.19 | Git operations (vendored OpenSSL) |
| `tempfile` 3 | Temp directory management |

### CI

Cross-platform CI via GitHub Actions — runs on every push to `main`:

- **Clippy** — lint-free (`-D warnings`)
- **Rustfmt** — format check
- **Tests** — Ubuntu, macOS, Windows (155 tests)
- **Release builds** — Linux x86_64, macOS x86_64, macOS ARM64, Windows x86_64

Release binaries are uploaded as artifacts on every green build.

---

## Contributing

Contributions welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

```bash
cargo test          # 155 tests must pass
cargo clippy        # must be warning-free
cargo fmt           # must be formatted
```

---

## License

MIT — see [LICENSE](LICENSE).

---

<div align="center">
  <strong>CodeDna</strong> — codebase intelligence for humans and AI agents 🧬
</div>
