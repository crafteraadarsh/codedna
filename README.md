<div align="center">

<img src="assets/banner-v2.svg" alt="CodeDna: codebase intelligence for humans and AI agents" width="100%">

<p>
  <a href="https://github.com/crafteraadarsh/codedna/releases/latest"><img src="https://img.shields.io/github/v/release/crafteraadarsh/codedna?style=flat-square&color=7ee787&label=release" alt="Latest release"></a>
  <a href="https://github.com/crafteraadarsh/codedna/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/crafteraadarsh/codedna/ci.yml?branch=main&style=flat-square&label=CI" alt="CI status"></a>
  <a href="https://github.com/crafteraadarsh/codedna/releases"><img src="https://img.shields.io/github/downloads/crafteraadarsh/codedna/total?style=flat-square&color=79c0ff" alt="Downloads"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/crafteraadarsh/codedna?style=flat-square&color=d2a8ff" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/built%20with-Rust-dea584?style=flat-square&logo=rust&logoColor=white" alt="Built with Rust">
  <img src="https://img.shields.io/badge/MCP-server-7ee787?style=flat-square" alt="MCP server">
</p>
<p>
  <a href="https://github.com/crafteraadarsh/codedna/stargazers"><img src="https://img.shields.io/github/stars/crafteraadarsh/codedna?style=flat-square&color=e3b341" alt="GitHub stars"></a>
  <a href="https://github.com/crafteraadarsh/codedna/commits/main"><img src="https://img.shields.io/github/last-commit/crafteraadarsh/codedna?style=flat-square" alt="Last commit"></a>
  <a href="https://github.com/crafteraadarsh/codedna/issues?q=is%3Aopen+label%3A%22good+first+issue%22"><img src="https://img.shields.io/github/issues/crafteraadarsh/codedna/good%20first%20issue?style=flat-square&color=7057ff&label=good%20first%20issues" alt="Good first issues"></a>
  <a href="https://github.com/crafteraadarsh/codedna/issues?q=is%3Aopen+label%3Ahacktoberfest"><img src="https://img.shields.io/badge/Hacktoberfest-friendly-ff8ae2?style=flat-square" alt="Hacktoberfest friendly"></a>
</p>

**Understand any codebase in milliseconds: its stack, architecture, entry points, dead code,<br>hotspots and supply-chain risks. One static binary, read-only and offline by default.**

[Install](#-install) · [Quick start](#-quick-start) · [AI agents](#-use-with-ai-agents) · [Commands](#-commands) · [Example](#-real-example) · [Security](#%EF%B8%8F-security) · [Contributing](#-contributing)

<br>

<img src="assets/demo-v2.svg" alt="Terminal demo: codedna analyze and codedna audit on a sample Express app" width="100%">

</div>

<br>

## ✨ Why CodeDna

<table>
<tr>
<td width="33%" valign="top">

### 🧬 Instant overview
Project type, languages, frameworks, databases, infrastructure and architecture from one command. Django's 5,700 files take about half a second.

</td>
<td width="33%" valign="top">

### 🤖 Built for AI agents
`codedna context .` prints a briefing sized for an agent's context window. Ships with an **MCP server**, a skill and a Claude Code plugin.

</td>
<td width="33%" valign="top">

### 🛡️ Safe on untrusted code
Read-only. It never runs scripts or installs anything from the repo, never opens `.env` files and sends nothing off your machine.

</td>
</tr>
<tr>
<td valign="top">

### 🔎 Supply-chain audit
`codedna audit` flags leaked secrets, risky install scripts, obfuscation, executables, hidden Unicode and typosquatted packages **before** you run a repo. Add `--deps` to also check lockfile versions against the [OSV](https://osv.dev) vulnerability database. Each finding has a severity and a confidence, and findings in tests, docs and examples are lowered. It's a list of red flags to check, not a safety verdict.

</td>
<td valign="top">

### 🔥 Hotspots and runbook
Find the files that are large *and* change often, and learn how to run a project: its scripts, env var names and API routes.

</td>
<td valign="top">

### 🌐 Local or remote
Pass a path or a Git URL. Remote repos are shallow-cloned to a temp folder and cleaned up afterwards. Output as text, JSON, Markdown or HTML.

</td>
</tr>
</table>

## 📦 Install

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/crafteraadarsh/codedna/main/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/crafteraadarsh/codedna/main/install.ps1 | iex
```

The script picks the right binary for your system, checks its SHA-256 checksum and installs it to `~/.local/bin` (Windows: `%LOCALAPPDATA%\codedna\bin`) without sudo. Set `CODEDNA_VERSION=v0.1.2` to pin a version or `CODEDNA_INSTALL_DIR` to choose the folder.

| Platform | Binary |
|---|---|
| Linux | x86_64 and arm64 (static, musl) |
| macOS | Intel and Apple Silicon |
| Windows | x86_64 |

<details>
<summary><b>Other ways to install, and verifying a download</b></summary>

<br>

**With Cargo** (needs [Rust stable](https://rustup.rs)):

```bash
cargo install --git https://github.com/crafteraadarsh/codedna
```

**From source:**

```bash
git clone https://github.com/crafteraadarsh/codedna
cd codedna
cargo install --path .
```

**Manual download:** grab an archive from [Releases](https://github.com/crafteraadarsh/codedna/releases). Every archive has a build-provenance attestation, so you can check it was built from this repository:

```bash
gh attestation verify codedna-x86_64-unknown-linux-musl.tar.gz --repo crafteraadarsh/codedna
```

Check the install with `codedna --version`.

</details>

## 🚀 Quick start

```bash
codedna analyze .                              # full intelligence report
codedna analyze https://github.com/user/repo   # works on remote repos too

codedna context .                              # short briefing for AI agents
codedna audit https://github.com/user/repo     # supply-chain red flags before you run it
codedna audit . --deps                         # plus known vulnerabilities in dependencies
codedna hotspots .                             # large files that change often
codedna runbook .                              # scripts, env var names, API routes

codedna report . --format html -o report.html  # shareable report (print to PDF)
codedna json . > before.json                   # save a snapshot...
codedna diff before.json .                     # ...and see what changed later
```

## 🤖 Use with AI agents

CodeDna works with any agent that can run a shell command, and connects natively to agents that support MCP or skills.

**1. Context in one command.** `codedna context .` prints a short Markdown briefing: project type, stack, architecture, entry points, most imported files, largest files and a directory map. `--max-tokens N` changes the budget; `--format json` gives the same data as JSON.

**2. MCP server.** `codedna mcp` serves read-only tools over stdio:

<p>
<code>get_context</code> · <code>analyze_repo</code> · <code>get_stack</code> · <code>list_files</code> · <code>find_framework_usage</code> · <code>find_dead_code</code> · <code>find_circular_imports</code> · <code>repo_map</code> · <code>find_hotspots</code> · <code>audit_repo</code> · <code>get_runbook</code>
</p>

| Agent | Setup |
|---|---|
| **Claude Code** | `claude mcp add codedna -- codedna mcp` |
| **Codex** | `codex mcp add codedna -- codedna mcp` |
| **OpenCode** | add to `opencode.json`: `"mcp": { "codedna": { "type": "local", "command": ["codedna", "mcp"] } }` |
| **Antigravity** | add to `~/.gemini/config/mcp_config.json`: `"mcpServers": { "codedna": { "command": "codedna", "args": ["mcp"] } }` |

The server only reads folders inside the directory it was started in. To allow others, pass them explicitly: `codedna mcp --root ~/code`. Remote repositories must be public `https://` URLs.

**3. Skill.** One skill file, [`skill/codedna/SKILL.md`](skill/codedna/SKILL.md), tells an agent when and how to use CodeDna.

| Agent | Where the skill goes |
|---|---|
| **Claude Code** | `.claude/skills/codedna/`, or install the plugin (below) |
| **Codex** | `~/.codex/skills/codedna/` (`./scripts/install-codex-skill.sh`) |
| **OpenCode** | `.opencode/skills/codedna/` or `~/.config/opencode/skills/codedna/` (`./scripts/install-opencode-skill.sh`) |
| **Antigravity** | `.agents/skills/codedna/` in your project |

To add the skill to another repository for several agents at once:

```bash
./scripts/bootstrap-agent-files.sh /path/to/repo claude opencode agents
```

**4. Claude Code plugin** (skill and MCP server together):

```text
/plugin marketplace add crafteraadarsh/codedna
/plugin install codedna@codedna-marketplace
```

All integrations need the `codedna` binary in your `PATH` (see [Install](#-install)).

<details>
<summary><b>Agent files in this repository</b></summary>

<br>

| File | Purpose |
|---|---|
| `AGENTS.md` | Instructions for OpenCode and Codex-style repo guidance |
| `CLAUDE.md` | Claude Code bootstrap for this repository |
| `GEMINI.md` | Gemini CLI bootstrap for this repository |
| `.cursorrules` | Cursor guidance for this repository |
| `.agents/skills/codedna/SKILL.md` | Skill for Antigravity and Codex |
| `.claude/skills/codedna/SKILL.md` | Skill for Claude Code |
| `.opencode/skills/codedna/SKILL.md` | Skill for OpenCode |
| `.claude-plugin/marketplace.json` | Claude marketplace definition |
| `plugins/codedna/` | Claude plugin: manifest, skill and MCP server |
| `skill/codedna/SKILL.md` | Canonical skill (CI checks every copy matches it) |
| `skill/codedna/agents/openai.yaml` | Codex UI metadata for the skill |
| `skills/codedna-analyzer/SKILL.md` | Full reference: JSON schema, detection tables, examples |
| `templates/CLAUDE.md` | Portable Claude bootstrap template |
| `scripts/` | Skill install, packaging and bootstrap scripts |

Each file tells the agent to run `codedna context .` (or call `get_context`) before a task and use the result to understand the project.

</details>

## 🧰 Commands

Every analysis command accepts a **local path** or a **Git URL**.

| Command | What it does |
|---|---|
| `codedna analyze [path\|url]` | Full intelligence report (`--time` adds elapsed time) |
| `codedna context [path\|url]` | Short briefing for AI agents (`--max-tokens N`, `--format json`) |
| `codedna audit [path\|url]` | Supply-chain red flags (`--json`, `--fail-on LEVEL` for CI, `--deps` for known vulnerabilities in dependencies) |
| `codedna hotspots [path\|url]` | Files ranked by commits × lines of code, plus files that change together (`--days N`, `--limit N`, `--json`) |
| `codedna runbook [path\|url]` | Scripts and targets, env var names (values are never read), API routes (`--json`) |
| `codedna report [path\|url] -o FILE` | Shareable Markdown report (`--format html` for a self-contained page) |
| `codedna diff <before> [after]` | Stack, size and file changes between two snapshots, paths or URLs (`--json`, `--markdown`) |
| `codedna json [path\|url]` | Full analysis as JSON (`--compact`, `--time`) |
| `codedna stack [path\|url]` | Languages, frameworks, databases |
| `codedna files [path\|url]` | Lines of code per file |
| `codedna framework <name> [path\|url]` | Files that import a given framework |
| `codedna deadcode [path\|url]` | Files nothing reaches from an entry point |
| `codedna cycles [path\|url]` | Groups of files that import each other (`--json`) |
| `codedna map [path\|url]` | Directory tree (`--depth N`) |
| `codedna scan [path\|url]` | Raw scanned file list |
| `codedna mcp [--root DIR]` | MCP server for AI agents (stdio) |

### Pull request comments

Add CodeDna to a repository's pull requests and each one gets a comment saying what it changes: lines of code, languages, frameworks added or removed, files added, removed or resized, and new possibly unused files. The comment is updated on every push instead of adding a new one.

```yaml
# .github/workflows/codedna.yml
name: CodeDna
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  diff:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: crafteraadarsh/codedna@main
```

| Input | Default | What it does |
|---|---|---|
| `version` | `latest` | CodeDna release to install |
| `binary` | | Path to a `codedna` binary to use instead |
| `comment` | `true` | `false` writes only the job summary |
| `github-token` | `github.token` | Token used to post the comment |

Pull requests from forks get a read-only token, so they get the job summary but no comment. Locally, `codedna diff before.json . --markdown` prints the same summary.

## 🔬 Real example

Output from CodeDna v0.1.1 on [`gothinkster/node-express-realworld-example-app`](https://github.com/gothinkster/node-express-realworld-example-app), an Express + Prisma API:

```text
  Summary
  ───────
  Total LOC          2,488
  Languages          5
  Frameworks         1
  Databases          1
  Infrastructure     1
  Files scanned      63
  Dead files         5
  Dependency links   49

  Project Type       Backend API service
  Stack              Express
  Databases          Prisma
  Infrastructure     Docker
  Architecture       API → Database
```

<details>
<summary><b>Full <code>analyze</code> report</b></summary>

<br>

```text
╔════════════════════════════════════════╗
║                                        ║
║      CodeDna Intelligence Report      ║
║                                        ║
╚════════════════════════════════════════╝

  Summary
  ───────
  Total LOC          2,488
  Languages          5
  Frameworks         1
  Databases          1
  Infrastructure     1
  Files scanned      63
  Dead files         5
  Dependency links   49

  Project Type
  ────────────
  Backend API service

  Stack
  ─────
  Express

  Databases
  ─────────
  Prisma

  Infrastructure
  ──────────────
  Docker

  Architecture
  ────────────
  API → Database

  Languages
  ─────────
  TypeScript        ███████████████████████░░░░░   84%      2,104 LOC
  JSON              ███░░░░░░░░░░░░░░░░░░░░░░░░░   13%        331 LOC
  Markdown          ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    1%         48 LOC
  TOML              ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0%          3 LOC
  JavaScript        ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0%          2 LOC

  Top 5 Files by LOC
  ──────────────────
  1  ./src/app/routes/article/article.service.ts              590 LOC   TypeScript
  2  ./src/app/routes/article/article.controller.ts           230 LOC   TypeScript
  3  ./src/tests/services/auth.service.test.ts                215 LOC   TypeScript
  4  ./src/app/routes/auth/auth.service.ts                    159 LOC   TypeScript
  5  ./src/tests/services/article.service.test.ts             126 LOC   TypeScript

  Dead Code
  ─────────
  e2e/src/support/global-setup.ts
  e2e/src/support/global-teardown.ts
  e2e/src/support/test-setup.ts
  jest.preset.js
  src/prisma/seed.ts
```

</details>

<details>
<summary><b><code>runbook</code>: how to run it</b></summary>

<br>

```text
  Runbook: 4 commands, 4 environment variables, 8 routes

  How to run (from the folder of each file)

    Dockerfile
      docker build .                 starts [ "node", "api" ]

    package.json
      npm run build                  nx build
      npm run start                  nx serve
      npm run test                   nx test

  Environment variables (names only; values are never read)

    HOST                           not in an example file · 1 read · e2e/src/support/test-setup.ts
    JWT_SECRET                     not in an example file · 3 reads · src/app/routes/auth/auth.ts, src/app/routes/auth/token.utils.ts
    NODE_ENV                       not in an example file · 1 read · src/prisma/prisma-client.ts
    PORT                           not in an example file · 2 reads · e2e/src/support/test-setup.ts, src/main.ts

  API routes

    GET     /articles                            src/app/routes/article/article.controller.ts:30
    POST    /articles                            src/app/routes/article/article.controller.ts:71
    POST    /users                               src/app/routes/auth/auth.controller.ts:14
    POST    /users/login                         src/app/routes/auth/auth.controller.ts:30
    GET     /user                                src/app/routes/auth/auth.controller.ts:45
    PUT     /user                                src/app/routes/auth/auth.controller.ts:61
    GET     /tags                                src/app/routes/tag/tag.controller.ts:13
    GET     /                                    src/main.ts:21
```

</details>

<details>
<summary><b><code>hotspots</code>: where change concentrates</b></summary>

<br>

```text
  Hotspots: last 3650 days, 50 commits

  Rank  Commits       LOC  File
  ────────────────────────────────────────
     1        2       590  src/app/routes/article/article.service.ts
     2       13        48  README.md
     3        3       159  src/app/routes/auth/auth.service.ts
     4        8        58  package.json
     5        2       230  src/app/routes/article/article.controller.ts
```

</details>

<details>
<summary><b>JSON output</b></summary>

<br>

`codedna json .` on the same project (`dependency_graph` and `file_breakdown` shortened):

```json
{
  "project_type": "Backend API service",
  "total_loc": 2488,
  "languages": {
    "JavaScript": 2,
    "JSON": 331,
    "TypeScript": 2104,
    "TOML": 3,
    "Markdown": 48
  },
  "frameworks": ["Express"],
  "databases": ["Prisma"],
  "infrastructure": ["Docker"],
  "architecture": "API → Database",
  "dead_code": [
    "e2e/src/support/global-setup.ts",
    "e2e/src/support/global-teardown.ts",
    "e2e/src/support/test-setup.ts",
    "jest.preset.js",
    "src/prisma/seed.ts"
  ],
  "dead_code_details": [
    { "file": "src/prisma/seed.ts", "confidence": "medium", "reason": "Nothing imports it, but \"seed\" is mentioned in 1 other file(s)" }
  ],
  "dependency_graph": { "...": ["..."] },
  "file_breakdown": [
    { "file": "./src/app/routes/article/article.service.ts", "loc": 590, "language": "TypeScript" }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `project_type` | string | High-level project classification |
| `total_loc` | number | Lines of code, excluding comments, blank lines and lockfiles |
| `comment_lines` | number | Comment-only lines across detected source files |
| `blank_lines` | number | Blank lines across detected source files |
| `languages` | object | Language → lines of code |
| `frameworks` | string[] | Detected frameworks |
| `databases` | string[] | Detected databases and ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Architecture summary from detected frameworks and databases |
| `architecture_details` | object | Layers found in the code (`layers`), import counts between them (`flows`), `confidence` (`high`/`medium`/`low`) and `evidence` |
| `dead_code` | string[] | Files nothing reaches from an entry point |
| `dead_code_details` | object[] | Per file `{ file, confidence, reason }`: how likely it is to be really unused, and why |
| `dependency_graph` | object | File → imported files |
| `file_breakdown` | object[] | Per-file `{ file, loc, language }` |

```bash
codedna json . --compact | jq -c '{project_type,frameworks,databases,architecture,total_loc}'
codedna json . | jq '.languages | to_entries | sort_by(-.value)'
codedna json . | jq '.dead_code[]'
```

</details>

## ⚙️ How it works

<p align="center"><img src="assets/architecture-v2.svg" alt="How CodeDna works: a path or Git URL goes through the Git handler and scanner, then languages, frameworks and the dependency graph are analyzed in parallel, dead code is found by reachability, and the combined result is shown in the terminal, as JSON, Markdown or HTML reports, or through MCP tools" width="100%"></p>

File-level work runs in parallel on every CPU core with [rayon](https://github.com/rayon-rs/rayon). Remote repositories are cloned into a temporary folder that is deleted when the run ends.

<details>
<summary><b>Git URLs</b></summary>

<br>

Any input starting with `https://` or `git@` is treated as a Git URL; everything else is a local path.

```text
https://github.com/user/repo
https://gitlab.com/user/repo
git@github.com:user/repo.git
```

Clones are shallow (`--depth 1`; up to 1,000 commits for `hotspots`). Plain `http://` URLs are refused, and a token in a URL (`https://user:token@host/...`) is replaced with `***` in all output. Missing repositories, network failures and private repositories that need authentication each get a clear error.

</details>

<details>
<summary><b>Detection coverage: 13 languages, 45 frameworks, 22 databases</b></summary>

<br>

Everything CodeDna recognises is defined in [`rules.toml`](rules.toml), so adding a detector is usually a one-line change.

**Languages:** Rust, TypeScript, JavaScript, Python, Go, Solidity, CSS, HTML, TOML, JSON, YAML, Markdown, Shell.

| Ecosystem | Frameworks |
|---|---|
| **JS / TS** | React, Next.js, Vue, Nuxt, Svelte, Astro, Remix, Gatsby, Express, Vite, NestJS, Fastify, Koa, Hapi |
| **Python** | FastAPI, Django, Flask, Starlette, Tornado, aiohttp, Pyramid, Sanic, Litestar |
| **Rust** | Tokio, Axum, Actix-web, Rocket, Warp, Tonic, Poem, Salvo, Tide, Hyper, Tauri, Leptos, Dioxus, Yew |
| **Go** | Gin, Echo, Fiber, Gorilla Mux, Beego, Chi, Revel, Uber FX |

| Manifest | Databases and ORMs |
|---|---|
| **`package.json`** | PostgreSQL, MongoDB, MySQL, Redis, Prisma, SQLite, Knex, Sequelize, TypeORM, Drizzle ORM, Cassandra, CouchDB, Elasticsearch, DynamoDB |
| **Python requirements** | SQLAlchemy, PostgreSQL, MongoDB, Redis, MySQL, Tortoise ORM, Peewee, Databases, Elasticsearch, Cassandra |
| **`Cargo.toml`** | SQLx, Diesel, SeaORM, PostgreSQL, MongoDB, Redis, SQLite, Sled, Elasticsearch |

| Signal | Infrastructure |
|---|---|
| `Dockerfile` | Docker |
| `docker-compose.yml` / `.yaml` | Docker Compose |
| `.github/workflows/*.yml` | GitHub Actions |
| `k8s/` or `kubernetes/` YAML | Kubernetes |
| `Makefile` | Makefile |

| Signals | Architecture |
|---|---|
| Frontend + backend + database | `Frontend → API → Database` |
| Frontend + backend | `Frontend → API` |
| Backend + database | `API → Database` |
| Frontend only | `Frontend only` |
| Backend only | `API only` |
| None detected | `Monolithic / undetermined` |

Paths matched by the repository's `.gitignore` files (root and nested) and `.ignore` files are skipped. `node_modules/`, `.git/`, `target/`, `dist/`, `build/` and `coverage/` are always skipped, as are files over 10 MiB and symbolic links.

</details>

<details>
<summary><b>Accuracy notes</b></summary>

<br>

- **Architecture** comes from two sources. The summary line uses detected frameworks and databases. `architecture_details` then checks the code: files are grouped into layers by folder and file name (`routes/`, `user.controller.ts`, `services/`, `models.py`, …), tests are left out, and imports between layers are counted. Confidence is high when layers import each other top-down, medium when the folders exist without supporting imports, and low when no layers are found.
- **Lines of code** count only lines that contain code. Comment-only and blank lines are reported separately (`comment_lines`, `blank_lines`), and comment markers inside strings are not treated as comments. Lockfiles (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock` and similar) are left out so they don't swamp the totals.
- **Dead code is a heuristic, with a confidence for each file.** A file is listed when nothing reaches it through imports from an entry point. Each one is then rated with a reason. *High*: its name appears nowhere else in the repository. *Medium*: nothing imports it, but its name is mentioned elsewhere or the repository loads modules dynamically. *Low*: it is probably loaded another way, for example from a convention folder (`migrations/`, `alembic/`, `management/`, `scripts/`, …), as a test, as a Django app module (`models.py`, `admin.py`, `signals.py`, …), or by a dynamic `import()`/`require()`/`importlib` call that names it. Python dotted strings in settings (`"app.views"`) count as imports. Check before deleting anything, even at high confidence.
- **Audit** matches known risky patterns. It cannot prove code is safe: a clean result only means none of its checks matched. Each finding has a severity (how bad it would be) and a confidence (how likely it is real, not a false alarm). Findings in tests, docs, examples and fixtures are lowered one step and marked with their `context`.
- **Monorepos**: workspaces declared at the repository root are listed with their packages (`workspaces` in JSON): npm, Yarn and Bun workspaces, pnpm (`pnpm-workspace.yaml`), Lerna, Cargo, Go (`go.work`) and uv. The repository is still analysed as a whole; there is no per-package breakdown yet.
- **Routes** in `runbook` come from common framework patterns and may be incomplete.

</details>

## ⚡ Performance

`codedna analyze` with v0.1.1 on a 2-core Linux machine, best of 5 runs:

| Repository | Files | Lines of code | Time |
|---|---:|---:|---:|
| [tokio](https://github.com/tokio-rs/tokio) | 886 | 173k | **51 ms** |
| [fastapi](https://github.com/fastapi/fastapi) | 2,993 | 256k | **163 ms** |
| [django](https://github.com/django/django) | 5,700 | 497k | **516 ms** |

## 🛡️ Security

CodeDna is built to be safe to run on any repository, including ones you do not trust.

| | |
|---|---|
| 🔒 **Read-only** | Never runs scripts, build steps, hooks or package managers from the repository. Git history is read without checking anything out. |
| 📴 **Nothing leaves your machine** | No telemetry or update checks. The only network access is cloning a URL you pass, and `audit --deps`, which sends lockfile package names and versions (nothing else) to osv.dev. |
| 🤫 **Secrets stay secret** | `.env` files are never opened. `runbook` reports variable names only, `audit` names the kind of credential and the line but never the value, and tokens in clone URLs are masked. |
| 🧹 **Hostile input is handled** | Terminal control and text-direction characters are stripped from repository text before printing; oversized files and snapshots are skipped. |
| 🚧 **The MCP server is fenced** | It only reads folders under its start directory (or ones you allow with `--root`) and public `https://` repositories, and every response is size-capped. |
| ✅ **Verifiable releases** | Binaries carry SHA-256 checksums and build-provenance attestations; dependencies are audited on every pull request. |

Found a vulnerability? Please report it privately as described in [SECURITY.md](SECURITY.md).

## 🤝 Contributing

Contributions are welcome, and CodeDna takes part in **Hacktoberfest**. Adding a framework or database detector is usually a one-line change to [`rules.toml`](rules.toml) plus a test, which makes it a great first PR.

- Browse [good first issues](https://github.com/crafteraadarsh/codedna/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
- Read the [contributing guide](CONTRIBUTING.md)
- Ask for a detector with the [detector request](https://github.com/crafteraadarsh/codedna/issues/new?template=detector_request.yml) template

```bash
cargo test     # all tests must pass
cargo clippy   # must be warning-free
cargo fmt      # must be formatted
```

Every pull request runs tests on Linux, macOS and Windows, plus Clippy, Rustfmt, a RustSec dependency audit and checks on the install scripts and skill copies.

<a href="https://github.com/crafteraadarsh/codedna/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=crafteraadarsh/codedna" alt="Contributors">
</a>

<details>
<summary><b>Project structure</b></summary>

<br>

```text
src/
├── main.rs                 entry point
├── cli.rs                  CLI definitions and command dispatch
├── analysis.rs             analysis pipeline
├── scanner.rs              file walker
├── language_detector.rs    extension → language
├── loc_counter.rs          line counting with binary detection
├── framework_detector.rs   manifest parsing and import scanning
├── dependency_graph.rs     import / require / mod parsing
├── dead_code_detector.rs   reachability from entry points
├── context.rs              token-budgeted briefing for agents
├── mcp.rs                  MCP server (JSON-RPC over stdio)
├── hotspots.rs             churn × size ranking, co-change coupling
├── audit.rs                supply-chain red-flag scan
├── vulns.rs                lockfile versions checked against OSV
├── runbook.rs              commands, env var names, API routes
├── diff.rs                 compare two snapshots or versions
├── report.rs               Markdown / HTML reports
├── reporter.rs             terminal report and JSON output
├── rules.rs                loads rules.toml
├── safe_text.rs            strips control characters from repo text
├── git_handler.rs          Git URLs, shallow clone, temp dirs
└── repo_map.rs             directory tree rendering
```

</details>

## 📄 License

[MIT](LICENSE) © CodeDna Contributors

<div align="center">
<br>
<sub><b>CodeDna</b> · codebase intelligence for humans and AI agents 🧬</sub>
<br>
<sub>If CodeDna saved you time, a ⭐ helps others find it.</sub>
</div>
