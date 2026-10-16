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
- Circular imports (files that import each other)
- Monorepo workspaces and their packages (npm, Yarn, Bun, pnpm, Lerna, Cargo, Go, uv), each with its size, languages, stack, dead files and the packages it uses and is used by
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
| "Are there circular imports?" | `codedna cycles .` |
| "Where is React used?" | `codedna framework react .` |
| "Show me the directory structure" | `codedna map .` |
| "Give me machine-readable output" | `codedna json .` |
| "Any red flags before I install this repo?" | `codedna audit <url>` |
| "How do I run this project?" | `codedna runbook .` |
| "Which env vars / API routes does it have?" | `codedna runbook .` |
| "How is this deployed? What runs where?" | `codedna topology .` |
| "What packages are in this monorepo?" | `codedna packages .` |
| "Which packages does a change to X affect?" | `codedna packages .` (see "Used by") |
| "What did this change do to the project?" | `codedna diff before.json .` |
| "What does my change affect? Which tests should I run?" | `codedna impact .` (branch: `--base origin/main`) |
| "Analyze this GitHub repo" | `codedna analyze <url>` |
| "What does this remote repo use?" | `codedna stack <url>` |

---

## Commands

All commands accept either a **local path** or a **Git URL**.

### Agent Briefing (start here)

```bash
codedna context .
codedna context . --max-tokens 1000
codedna context . --format json
```

A short Markdown briefing sized for a context window: project type, architecture, size, stack, entry points, most imported files, largest files and a trimmed directory map. Default budget: 2000 tokens (minimum 300).

### Shareable Report

```bash
codedna report . -o report.md
codedna report . --format html -o report.html
```

A document for people: summary, stack, languages, entry points, most imported and largest files, hotspots, possibly unused files (up to 50) and a directory map. The HTML file is self-contained (no scripts or external resources) and has a print stylesheet, so a browser can save it as PDF. Everything taken from the repository is escaped.

### Hotspots

```bash
codedna hotspots .
codedna hotspots . --days 90 --limit 20 --json
```

Ranks source files by the number of Git commits that changed them in the last N days (default 365) multiplied by their lines of code. Large files that change often are where bugs and refactoring effort usually gather. Merge commits are skipped, renames are not followed, and at most 5,000 commits are read. History is read from Git objects only; nothing is checked out and no hooks or filters run. A remote URL is cloned with up to 1,000 commits of history.

The same history gives **files that change together** (co-change coupling): source-file pairs changed in the same commit at least 3 times, where that covers at least half of the less active file's commits. These often point to a hidden dependency: a change to one usually needs a change to both. Commits touching more than 30 files are skipped for coupling, so bulk renames and formatting passes don't pair everything. The top 10 pairs are shown, and in JSON they are under `coupling`.

### Supply-Chain Audit

```bash
codedna audit .
codedna audit https://github.com/user/repo --json
codedna audit . --fail-on high
codedna audit . --deps
codedna audit . --write-baseline audit-baseline.json
codedna audit . --baseline audit-baseline.json --fail-on high
```

A scan for red flags that often show up in malicious or compromised repositories:

| Check | What it flags | Severity |
|---|---|---|
| `secret` | Hardcoded AWS, GitHub, GitLab, Slack, Stripe, Google, npm, PyPI, OpenAI and Anthropic keys, and private keys. The value is never shown. | high |
| `install-script` | npm `preinstall`/`install`/`postinstall` scripts (high if they download or run code), network or shell calls in `setup.py`, network access in `build.rs` | low–high |
| `obfuscation` | Decoded data passed to `eval`/`exec` on one line, long hex-escape strings, large encoded blobs | low–high |
| `binary` | Checked-in ELF, Mach-O or Windows executables | medium |
| `hidden-unicode` | Bidirectional controls (Trojan Source) and zero-width characters in code | low–high |
| `minified` | Very long lines in non-`.min` source files | low |
| `typosquat` | npm or PyPI dependencies one edit away from a popular package | medium |
| `vulnerable-dependency` | With `--deps`: a locked dependency version with published advisories in [OSV](https://osv.dev), with the advisory ids and the version that fixes them | low–high |

The risk score adds 25 per high, 10 per medium and 2 per low finding, capped at 100. `--fail-on low|medium|high` exits with status 1 when a finding reaches that level, for CI. Each finding also has a `confidence` (`high`, `medium`, `low`): how likely it is a real problem rather than a false alarm. Findings in tests, docs, examples, fixtures, vendored libraries and translations are lowered one step on severity and confidence and carry a `context` field (`test`, `docs`, `example`, `fixture`, `vendored`, `translation`); `lowered` counts them. Keys in any of them drop to low confidence, and long lines, encoded data and direction marks in tests, fixtures, vendored libraries and translations are not reported. The JSON includes a `note`. Findings are leads to review, not a safety verdict: clean code can trip a check, a careful attacker can avoid them all, and a clean result does not prove the code is safe.

`--deps` (MCP: `check_dependencies: true`) is the only part of the audit that uses the network, so it is off by default. It reads `Cargo.lock`, `package-lock.json`, `npm-shrinkwrap.json`, `yarn.lock`, `pnpm-lock.yaml`, `requirements*.txt` (`==` pins), `poetry.lock`, `uv.lock`, `Pipfile.lock`, `go.mod`, `Gemfile.lock` and `composer.lock` (not inside `node_modules`, `vendor` or `target`), and sends package names and versions, nothing else, to osv.dev. Each vulnerable package is one finding: its severity is the worst advisory's (GitHub's rating, else the CVSS score; RustSec "unmaintained" notices are low). The JSON adds `dependencies` with `packages_checked`, `lockfiles` and an `error` if the database could not be reached; with `--fail-on`, that exits with status 2 so a CI gate cannot pass unchecked. A matched version means the advisory applies to that release, not that the vulnerable code is reachable from this project.

**Baselines.** `--write-baseline FILE` records every current finding (before the 200-finding cap) as accepted; `--baseline FILE` then hides those and reports only new ones, and `--fail-on` applies to the new ones only. Entries are matched on check, file and message, not line, so edits elsewhere in a file don't break them; each entry hides one finding, so a second identical finding in the same file is still reported. Add a `"reason"` to any entry: writing the baseline again keeps it, and drops entries that no longer match. The text output and the JSON `baseline` field give `hidden` (findings hidden) and `stale` (entries that matched nothing). A missing or invalid baseline exits with status 2. CodeDna never reads a baseline it finds in the scanned repository, only the file you name, so a repository cannot hide its own findings.

### Packages

```bash
codedna packages .
codedna packages https://github.com/user/repo --json
```

Lists the packages of each monorepo workspace declared at the root. For each one: lines of code, files, main languages, frameworks and databases found in its own manifests, dead-code candidates, the other packages it uses (manifest dependencies and imports across package folders) and the packages that use it. A file belongs to the deepest package folder that holds it. `--json` prints the `workspaces` array. To analyse one package fully, run `codedna analyze <package path>`.

### Impact

```bash
codedna impact .                      # uncommitted changes
codedna impact . --base origin/main   # a branch, from where it left main
codedna impact . --markdown           # for a pull request comment
```

Lists the changed files (untracked ones count as added), every file that imports them directly or through other files (nearest first, with the import that brings each in), the architecture layers involved, API routes declared in those files, and the test files to run. Local Git repositories only. Imports the dependency graph does not follow are not seen.

### Runbook

```bash
codedna runbook .
codedna runbook https://github.com/user/repo --json
```

How to run a project, read from its files:

- **Commands**: `package.json` scripts (with npm, pnpm, yarn or bun picked from the lockfile), Make and just targets, Dockerfiles (exposed ports and start command), Compose services, Cargo binaries, `pyproject.toml` scripts, Django `manage.py` and Procfile entries.
- **Environment variables**: names read by code (`process.env`, `import.meta.env`, `os.getenv`, `os.environ`, `std::env::var`, Go `os.Getenv`, Ruby `ENV`, PHP `getenv`, Java `System.getenv`, .NET `GetEnvironmentVariable`, Deno and Bun) and names listed in example files such as `.env.example`. Each is marked as documented or not.
- **API routes**: Express, Fastify, Hono, NestJS, Next.js (`app/**/route.ts` and `pages/api`), Flask, FastAPI, Django, Spring, actix, Rocket, axum, Gin, Echo, chi, net/http, Laravel and Rails.

Nothing is run. Environment variables are reported by name only: `.env` files are never opened, example files contribute only the text before `=`, and a command that looks like it contains a credential is hidden. Routes come from common framework patterns, so the list can be incomplete.

### Topology

```bash
codedna topology .
codedna topology https://github.com/user/repo --json
```

What the deployment files say will run, read without rendering or deploying anything:

- **Platforms**: Kubernetes manifests (Deployment, StatefulSet, DaemonSet, Job, CronJob, Pod, Knative Service), Helm charts (templates read as written), Skaffold, Docker Compose, Serverless Framework, AWS SAM and CloudFormation Lambda functions, Vercel, Netlify, Cloudflare Workers, Fly.io, Render, App Engine and Procfile.
- **Per unit**: image, command, runtime and handler, ports, replicas, autoscaling (HPA, KEDA), schedule, and what reaches it: Services, Ingress and Gateway API routes, HTTP, queue, stream and schedule events.
- **Code**: the folder or file behind each unit, from Skaffold and Compose build contexts, an image named like a folder with a Dockerfile, or a function handler.
- Files in test, example and docs folders are skipped and counted. YAML is read with depth and alias limits, so hostile files cannot exhaust memory.

### Diff

```bash
codedna json . > before.json   # save a snapshot
codedna diff before.json .     # compare it with the folder now
codedna diff old.json new.json --json
```

Compares two versions of a project. Each side is a snapshot saved with `codedna json`, a folder or a Git URL (the second defaults to `.`). It shows lines of code and file counts before and after, project type and architecture changes, frameworks, databases and infrastructure added or removed, per-language line changes, files added, removed or resized (25 of each, largest first, with totals) and files that newly look unused. Snapshot files over 64 MB are refused, and every name read from one is sanitized before it is printed.

### MCP Server

```bash
codedna mcp
codedna mcp --root ~/code
```

Serves read-only tools over stdio (JSON-RPC 2.0): `get_context`, `analyze_repo`, `get_stack`, `list_files`, `find_framework_usage`, `find_dead_code`, `find_circular_imports`, `repo_map`, `find_hotspots`, `audit_repo`, `get_runbook`, `get_topology`, `list_packages`, `get_impact`. Local paths must be inside the start directory or a `--root`; remote repositories must be public `https://` URLs. Output is size-capped, and file names are returned as data.

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

Returns list of files unreachable from any entry point via dependency graph traversal. `--write-baseline FILE` accepts the current candidates and `--baseline FILE` hides them (the same file can hold both audit and dead-code entries; writing one section keeps the other).

### Circular Imports

```bash
codedna cycles [path or url]
codedna cycles [path or url] --json
```

Lists groups of files that import each other, each shown as the shortest loop, e.g. `src/a.ts → src/b.ts → src/a.ts`. Uses the same dependency graph as dead code detection, and type-only imports count, so treat each one as something to check rather than a confirmed runtime problem. `codedna diff` reports cycles that a change introduces.

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
  "circular_imports": [
    ["src/api/routes.ts", "src/controllers/user.ts"]
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
| `total_loc` | number | Lines of code, excluding comments, blank lines and lockfiles |
| `languages` | object | Language -> LOC count map |
| `frameworks` | string[] | Detected application frameworks |
| `databases` | string[] | Detected databases / ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Inferred architecture pattern |
| `dead_code` | string[] | Unreachable file paths |
| `dead_code_details` | object[] | Per file `{ file, confidence, reason, evidence }`; confidence is `high`, `medium` or `low`; `evidence` lists up to 3 `{ file, line, text }` lines behind the reason |
| `dependency_graph` | object | File -> imported files map |
| `coverage` | object | How much the dependency analysis covered: `source_files`, `analyzed_files` (imports followed), `unreadable_files`, `not_analyzed` (languages whose imports are not followed, `{ language, files }`), `imports` (`{ internal, resolved, unresolved, external }`), `unresolved` (up to 20 `{ file, line, import }`) and a one-line `summary`. Read it before trusting an empty `dead_code` or `circular_imports` |
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
| `requirements.txt` / `pyproject.toml` | SQLAlchemy, SQLModel, PostgreSQL, MongoDB, Redis, MySQL, Tortoise ORM, Peewee |
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
| Repeat runs | Incremental — unchanged files (same size and modification time) are not read again |

**Cache.** Per-file results (line counts, imports, layer, dead-code mentions) are kept in the user cache folder: `$XDG_CACHE_HOME/codedna` or `~/.cache/codedna`, `~/Library/Caches/codedna` on macOS, `%LOCALAPPDATA%\codedna\cache` on Windows, or `CODEDNA_CACHE_DIR`. Output is identical with and without it. Imports are reused only while the file list and every `.json` / `.toml` file are unchanged; dead-code mentions only for the same candidates. The cache is never stored in or read from the scanned repository, remote clones are never cached, and a cache from another CodeDna version is ignored. `--no-cache` (any command) or `CODEDNA_NO_CACHE=1` turns it off.

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
