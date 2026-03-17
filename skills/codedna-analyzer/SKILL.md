# CodeDna Analyzer Skill

```yaml
name: codedna-analyzer
version: 1.1.0
description: >
  Analyze any local or remote repository and reveal its codebase DNA —
  tech stack, architecture, LOC distribution, framework usage,
  infrastructure, and dead code. Outputs structured JSON for agent consumption.
tools:
  - codedna
repository: https://github.com/crafteraadarsh/codedna
compatibility:
  - codex
  - claude
  - opencode
  - cursor
  - gemini
```

---

## Purpose

CodeDna Analyzer is a **codebase intelligence skill** for AI coding agents and developers. Before reading a single source file, an agent can run CodeDna to instantly understand what it's working with.

It detects and reports:

- Programming languages and LOC distribution (with visual bar chart)
- Frameworks across JS/TS, Python, Rust, and Go ecosystems
- Databases and data-layer technologies
- Infrastructure and DevOps tooling (Docker, GitHub Actions, Kubernetes)
- Architecture patterns (Frontend -> API -> Database)
- Per-file framework import usage
- Unused / dead code via dependency graph traversal
- Directory tree structure

Supports both **local directories** and **remote Git repositories** (GitHub, GitLab, Bitbucket, etc.).

---

## Installation

```bash
# Install from GitHub
cargo install --git https://github.com/crafteraadarsh/codedna

# Or from source
git clone https://github.com/crafteraadarsh/codedna
cd codedna
cargo install --path .
```

Requires **Rust stable** — install at [rustup.rs](https://rustup.rs).

Verify:

```bash
codedna --version
```

---

## When To Use This Skill

Trigger this skill when the user asks any of the following:

| User Intent | Command |
|---|---|
| "What tech stack does this repo use?" | `codedna stack .` |
| "Explain this repository" | `codedna analyze .` |
| "Show the architecture of this project" | `codedna analyze .` |
| "How many lines of code?" | `codedna files .` |
| "Find unused or dead code" | `codedna deadcode .` |
| "Where is React used?" | `codedna framework react .` |
| "Show me the directory structure" | `codedna map .` |
| "Give me machine-readable output" | `codedna json .` |
| "Analyze this GitHub repo" | `codedna analyze <url>` |
| "What does this remote repo use?" | `codedna stack <url>` |

---

## Commands

All commands accept either a **local path** or a **Git URL**.

### Full Intelligence Analysis

```bash
codedna analyze [path or url]
codedna analyze [path or url] --time
```

Produces the complete formatted intelligence report: project type, stack, databases, infrastructure, architecture, LOC bar chart, top 5 files, dead code, and summary.

### Tech Stack Only

```bash
codedna stack [path or url]
```

Returns detected languages (with LOC %), frameworks, and databases.

### File-Level LOC Breakdown

```bash
codedna files [path or url]
```

Returns per-file LOC breakdown sorted by size descending.

### Framework Usage (per-file)

```bash
codedna framework <name> [path or url]
```

Returns all files where the specified framework is imported/used.

Examples:

```bash
codedna framework react .
codedna framework fastapi ./backend
codedna framework axum https://github.com/user/rust-project
```

### Dead Code Detection

```bash
codedna deadcode [path or url]
```

Returns list of files unreachable from any entry point via dependency graph traversal.

### Repository Map

```bash
codedna map [path or url]
codedna map [path or url] --depth 3
```

Renders a tree-style directory structure respecting ignore rules.

### Raw Scan

```bash
codedna scan [path or url]
```

Returns the raw scanned file list.

### Machine-Readable JSON (Agent Use)

```bash
codedna json [path or url]
codedna json [path or url] --compact
codedna json [path or url] --time
```

Returns the full analysis as structured JSON. Intended for agent consumption.

---

## Git URL Support (v1.1)

CodeDna automatically detects Git URLs and handles cloning transparently.

### Supported URL Formats

Any input starting with `http://`, `https://`, or `git@` is treated as a Git URL. Everything else is treated as a local path.

```
https://github.com/user/repo
https://gitlab.com/user/repo
http://github.com/user/repo
git@github.com:user/repo.git
git@gitlab.com:user/repo.git
```

### How It Works

1. Detect whether input is a local path or a Git URL
2. If local path -> analyze directly
3. If Git URL -> shallow-clone (`--depth 1`) into a temp directory
4. Run the standard analysis pipeline
5. Delete the temp directory automatically

### Examples

```bash
# Analyze a public GitHub repo
codedna analyze https://github.com/vercel/next.js

# Get JSON output for a remote repo
codedna json https://github.com/tokio-rs/tokio --compact

# Check dead code in a remote repo
codedna deadcode https://github.com/denoland/deno

# SSH URL
codedna analyze git@github.com:user/repo.git
```

### Error Handling

User-friendly messages for:

- Invalid or malformed Git URL
- Clone failure (auth, network, non-existent repo)
- Network timeout or connectivity issues

---

## JSON Schema

`codedna json .` produces:

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
  "architecture": "Frontend -> API -> Database",
  "dead_code": [
    "src/utils/oldHelper.ts",
    "src/components/LegacyNavbar.tsx"
  ],
  "dependency_graph": {
    "src/server.ts": ["src/api/routes.ts"],
    "src/api/routes.ts": ["src/controllers/user.ts"]
  },
  "file_breakdown": [
    { "file": "src/server.ts", "loc": 340, "language": "TypeScript" },
    { "file": "src/App.tsx", "loc": 210, "language": "TypeScript" }
  ]
}
```

### Field Reference

| Field | Type | Description |
|---|---|---|
| `project_type` | string | High-level project classification |
| `total_loc` | number | Total non-empty lines of code |
| `languages` | object | Language -> LOC count map |
| `frameworks` | string[] | Detected application frameworks |
| `databases` | string[] | Detected databases / ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Inferred architecture pattern |
| `dead_code` | string[] | Unreachable file paths |
| `dependency_graph` | object | File -> imported files map |
| `file_breakdown` | object[] | Per-file `{ file, loc, language }` entries |

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

---

## Architecture Inference

| Signal Combination | Output |
|---|---|
| Frontend + Backend + Database | `Frontend -> API -> Database` |
| Frontend + Backend | `Frontend -> API` |
| Backend + Database | `API -> Database` |
| Frontend + Database (no backend) | `Frontend -> Database` |
| Frontend only | `Frontend only` |
| Backend only | `API only` |
| None of the above | `Monolithic / undetermined` |

---

## Project Type Inference

| Detected Signals | Project Type |
|---|---|
| React/Vue/Next.js + Express/FastAPI + DB | Full-stack web application |
| React/Vue/Next.js only | Frontend web application |
| Axum/Tokio + DB + Rust | Rust backend service |
| FastAPI/Django/Flask + DB + Python | Python backend service |
| Gin/Echo + Go | Go backend service |
| Solidity present | Blockchain / Smart-contract project |
| Rust only | Rust library / CLI tool |
| Docker present (no framework) | Containerised service |

---

## Performance

| Metric | Value |
|---|---|
| Local analysis (26 files, debug build) | ~2.6 ms |
| Remote clone | Shallow (`--depth 1`) — only latest commit |
| Parallelism | `rayon` thread pool — all CPU cores |
| Target for 100k+ file repos | < 5 seconds |
| Memory model | Streaming per-file reads, no full-repo buffer |
| Temp dir cleanup | Automatic — deleted immediately after analysis |

---

## Useful jq Recipes

```bash
# Project type and architecture only
codedna json . | jq '{project_type, architecture}'

# List all detected frameworks
codedna json . | jq '.frameworks[]'

# Languages sorted by LOC descending
codedna json . | jq '.languages | to_entries | sort_by(-.value)'

# Dead code files only
codedna json . | jq '.dead_code[]'

# Top 5 largest files
codedna json . | jq '.file_breakdown[:5]'

# Check if Docker is in use
codedna json . | jq '.infrastructure | contains(["Docker"])'

# Minimal agent context (compact, single line)
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'

# Full dependency graph
codedna json . | jq '.dependency_graph'
```

---

## Ignored Directories

The scanner always skips:

```
node_modules/   .git/   target/   dist/   build/   coverage/
```

---

## Links

- **Repository:** https://github.com/crafteraadarsh/codedna
- **Install:** `cargo install --git https://github.com/crafteraadarsh/codedna`
- **Issues:** https://github.com/crafteraadarsh/codedna/issues
- **Contributing:** See [CONTRIBUTING.md](../../CONTRIBUTING.md)
