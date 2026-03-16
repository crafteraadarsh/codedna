# CodeDna 🧬

> Rust-powered codebase intelligence engine and agent skill.

[![Build](https://img.shields.io/badge/build-passing-brightgreen)](#)
[![Tests](https://img.shields.io/badge/tests-140%20passing-brightgreen)](#)
[![License](https://img.shields.io/badge/license-MIT-blue)](#license)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](#)

CodeDna analyzes any repository and reveals its **DNA** — the tech stack, architecture, LOC distribution, framework usage, infrastructure, and dead code — in milliseconds.

---

## What It Does

Given any repository, CodeDna produces a complete intelligence report:

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
git clone https://github.com/your-org/codedna
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

| Command | Description |
|---|---|
| `codedna analyze [path]` | Full intelligence report |
| `codedna analyze [path] --time` | Report + elapsed time |
| `codedna stack [path]` | Languages, frameworks, databases |
| `codedna files [path]` | Per-file LOC breakdown |
| `codedna framework <name> [path]` | Files that import the given framework |
| `codedna deadcode [path]` | Unused / unreachable files |
| `codedna map [path]` | Directory tree |
| `codedna map [path] --depth N` | Directory tree limited to N levels |
| `codedna scan [path]` | Raw scanned file list |
| `codedna json [path]` | Full analysis as JSON |
| `codedna json [path] --compact` | Single-line JSON |
| `codedna json [path] --time` | JSON + elapsed time |

### Framework Detection

The `framework` command supports per-file import scanning for:

**JavaScript / TypeScript:** React, Next.js, Express, Vite, Vue, Svelte, NestJS, Fastify, Koa, Remix, Gatsby, Nuxt, Astro

**Python:** FastAPI, Django, Flask, Starlette, aiohttp

**Rust:** Tokio, Axum, Actix-web, Rocket, Warp, Leptos, Dioxus, Yew, Tauri

**Go:** Gin, Echo, Fiber, Chi

```bash
codedna framework react .
codedna framework fastapi ./backend
codedna framework axum .
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
Repository
    │
    ▼
Scanner (walkdir)          — recursive walk, ignore node_modules/.git/
    │
    ├──► Language Detector  — file extension → Language enum
    ├──► LOC Counter        — non-empty lines, skip binary/UTF-8 errors  (parallel)
    ├──► Framework Detector — parse package.json / requirements.txt / Cargo.toml / go.mod
    ├──► Infra Detector     — detect Dockerfile / docker-compose / .github/workflows
    ├──► Dependency Graph   — parse import/require/mod statements         (parallel)
    └──► Dead Code Detector — BFS from entry points over dependency graph
                │
                ▼
          AnalysisResult
                │
          ┌─────┴─────┐
          ▼           ▼
    CLI Report     JSON Output
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
| Analysis time (26 files, debug build) | ~2.6 ms |
| Parallelism | `rayon` thread pool — all CPU cores |
| Target for 100 000+ file repos | < 5 seconds |
| Memory model | Streaming per-file reads, no full-repo buffer |

Build in release mode for maximum performance:

```bash
cargo build --release
./target/release/codedna analyze .
```

---

## AI Agent Integration

CodeDna is designed to be the **first thing any AI agent runs on a new codebase**.

```bash
# Give your agent instant codebase context
codedna json . --compact | jq -c \
  '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'
```

See [`../Skill.md`](../Skill.md) for the complete agent integration guide including
Claude Code, OpenCode, Codex CLI, Cursor, and Gemini CLI workflows.

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