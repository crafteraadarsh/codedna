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
Project type, languages, frameworks, databases, infrastructure, architecture layers, circular imports and monorepo workspaces from one command. Django's 5,700 files take about 0.6 seconds.

</td>
<td width="33%" valign="top">

### 🤖 Built for AI agents
`codedna context .` prints a briefing sized for an agent's context window. Ships with an **MCP server**, a skill and a Claude Code plugin.

</td>
<td width="33%" valign="top">

### 🛡️ Safe on untrusted code
Read-only. It never runs scripts or installs anything from the repo, never opens `.env` files and sends nothing off your machine unless you ask for `audit --deps`.

</td>
</tr>
<tr>
<td valign="top">

### 🔎 Supply-chain audit
`codedna audit` flags leaked secrets, risky install scripts, obfuscation, executables, hidden Unicode and typosquatted packages **before** you run a repo. Add `--deps` to also check lockfile versions against the [OSV](https://osv.dev) vulnerability database. Each finding has a severity and a confidence, and findings in tests, docs and examples are lowered. A baseline accepts the findings you already have, so CI fails only on new ones. It's a list of red flags to check, not a safety verdict.

</td>
<td valign="top">

### 🔥 Hotspots, runbook and deployment
Find the files that are large *and* change often, learn how to run a project (its scripts, env var names and API routes), and see what its Kubernetes, Helm, Compose or serverless files say will run.

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

The script picks the right binary for your system, checks its SHA-256 checksum and installs it to `~/.local/bin` (Windows: `%LOCALAPPDATA%\codedna\bin`) without sudo. Set `CODEDNA_VERSION=v0.2.11` to pin a version or `CODEDNA_INSTALL_DIR` to choose the folder.

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
codedna context . --task "add rate limiting"     # files that matter for a task
codedna audit https://github.com/user/repo     # supply-chain red flags before you run it
codedna audit . --deps                         # plus known vulnerabilities in dependencies
codedna audit . --baseline audit.json --fail-on high   # in CI: fail only on new findings
codedna hotspots .                             # large files that change often
codedna runbook .                              # scripts, env var names, API routes
codedna topology .                             # what deployment files run, how it is reached, which code
codedna packages .                             # monorepo packages: size, stack, who uses whom
codedna impact . --base origin/main            # what your branch affects: importers, layers, routes, tests
codedna explain src/app.ts .                   # why a file counts as used or dead, with the import chain
codedna query references src/db.ts .           # who imports a file, each with its line

codedna report . --format html -o report.html  # shareable report (print to PDF)
codedna json . > before.json                   # save a snapshot...
codedna diff before.json .                     # ...and see what changed later
```

## 🤖 Use with AI agents

CodeDna works with any agent that can run a shell command, and connects natively to agents that support MCP or skills.

**1. Context in one command.** `codedna context .` prints a short Markdown briefing: project type, stack, architecture, entry points, most imported files, largest files and a directory map. `--max-tokens N` changes the budget; `--format json` gives the same data as JSON. `--task "add rate limiting to login"` opens the briefing with the files that matter for that task: files whose names or code mention its words, the files they import or are imported by, and related tests, each with why it was picked (MCP: the `task` argument of `get_context`). A neighbour linked only by a type import is marked `(types only)` and counts half; a file the program does not confirm as used has a `usage` such as `type_only` or `unreachable`. Matching is by wording, not meaning. Each file carries `evidence` (the line of its first mention of each word, or the import line that links it to a match), and a `limits` block says what bounded the answer: how many files matched against how many are shown, task words nothing mentions, files searched by path only (prose, data, or code over 512 KiB) and how much of the import graph was followed. The Markdown briefing sums this up in a `Limits:` line. When most of the source is in a language CodeDna does not read, the briefing says the architecture is unknown, and it lists up to three readings the layers leave out (`architecture_unknown` and `competing_interpretations` in JSON).

**2. MCP server.** `codedna mcp` serves read-only tools over stdio:

<p>
<code>get_context</code> · <code>analyze_repo</code> · <code>get_stack</code> · <code>list_files</code> · <code>find_framework_usage</code> · <code>find_dead_code</code> · <code>find_circular_imports</code> · <code>repo_map</code> · <code>find_hotspots</code> · <code>audit_repo</code> · <code>get_runbook</code> · <code>get_topology</code> · <code>list_packages</code> · <code>get_impact</code> · <code>explain</code> · <code>query</code>
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
| `codedna context [path\|url]` | Short briefing for AI agents (`--max-tokens N`, `--format json`, `--task "…"`) |
| `codedna audit [path\|url]` | Supply-chain red flags (`--json`, `--fail-on LEVEL` for CI, `--deps` for known vulnerabilities in dependencies, `--baseline FILE` / `--write-baseline FILE` to report only new findings) |
| `codedna hotspots [path\|url]` | Files ranked by commits × lines of code, plus files that change together (`--days N`, `--limit N`, `--json`) |
| `codedna runbook [path\|url]` | Scripts and targets, env var names (values are never read), API routes (`--json`) |
| `codedna topology [path\|url]` | What the deployment files say will run: Kubernetes, Helm, Skaffold, Docker Compose, Serverless Framework, AWS SAM and CloudFormation, Vercel, Netlify, Cloudflare Workers, Fly.io, Render, App Engine and Procfile. Each unit with its image, ports, replicas, schedule, what reaches it (Services, Ingress, HTTP and queue events) and the code it runs (`--json`) |
| `codedna packages [path\|url]` | Monorepo packages with size, languages, stack, dead files and the packages each one uses and is used by (`--json`) |
| `codedna impact [path]` | What a change affects: changed files, the files that import them, layers, API routes and tests to run. Compares uncommitted work with `HEAD`, or a branch with `--base origin/main` from where it left it (`--json`, `--markdown`) |
| `codedna explain <file\|id> [path]` | Why a file is used, unused or in a cycle: its entry point and why that counts as one, the import chain that reaches it with the line of each step, its dead-code finding, importers, imports, the assumptions behind the answer and provenance (analyzer and rules version, git revision, snapshot hash). Accepts a file path, `dead:<file>` or `cycle:<file>` (`--json`) |
| `codedna query <op> … [path]` | Import facts with evidence: `references <file>` (who imports it), `imports <file>`, `path <from> <to>` (shortest import chain), `entrypoints` (each root with its reason), `unresolved [--file F]` (imports that did not resolve). `--json`, `--limit N` (default 50) |
| `codedna report [path\|url] -o FILE` | Shareable Markdown report (`--format html` for a self-contained page); lists its assumptions and limits, and each possibly unused file with its status and an `explain` command |
| `codedna diff <before> [after]` | Stack, size and file changes between two snapshots, paths or URLs (`--json`, `--markdown`) |
| `codedna json [path\|url]` | Full analysis as JSON (`--compact`, `--time`) |
| `codedna schema` | JSON Schema of the `json` report |
| `codedna calibration` | How often each confidence level was right on hand-checked repositories, and whether `high` passed the release gate (`--json`, `--set NAME`, `--role development\|calibration\|sealed`, `--corpus DIR` to re-measure this build, `--fixtures` for the annotated fixtures) |
| `codedna stack [path\|url]` | Languages, frameworks, databases |
| `codedna files [path\|url]` | Lines of code per file |
| `codedna framework <name> [path\|url]` | Files that import a given framework |
| `codedna deadcode [path\|url]` | Files nothing reaches from an entry point (`--baseline FILE` / `--write-baseline FILE` to hide accepted ones) |
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
      - uses: crafteraadarsh/codedna@v0.2.11
```

| Input | Default | What it does |
|---|---|---|
| `version` | `latest` | CodeDna release to install |
| `binary` | | Path to a `codedna` binary to use instead |
| `comment` | `true` | `false` writes only the job summary |
| `github-token` | `github.token` | Token used to post the comment |

Pull requests from forks get a read-only token, so they get the job summary but no comment. Locally, `codedna diff before.json . --markdown` prints the same summary.

## 🔬 Real example

Output from CodeDna v0.1.3 on [`gothinkster/node-express-realworld-example-app`](https://github.com/gothinkster/node-express-realworld-example-app), an Express + Prisma API:

```text
  Summary
  ───────
  Total LOC          2,243
  Comment lines      245
  Languages          5
  Frameworks         1
  Databases          1
  Infrastructure     1
  Files scanned      63
  Dead files         5  (2 medium, 3 low)
  Circular imports   1
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
  Total LOC          2,243
  Comment lines      245
  Languages          5
  Frameworks         1
  Databases          1
  Infrastructure     1
  Files scanned      63
  Dead files         5  (2 medium, 3 low)
  Circular imports   1
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
  Code layers: Routes → Services → Data  (confidence: high)
    · Routes: 11 files in src/app/routes/auth/, src/app/routes/article/, src/app/routes/profile/
    · Services: 4 files in src/app/routes/article/, src/app/routes/auth/, src/app/routes/profile/
    · Data: 10 files in src/app/routes/auth/, src/app/routes/article/, src/prisma/
    · Services → Data: 12 imports
    · Routes → Services: 4 imports
    · Services → Routes: 4 imports
    · e.g. Routes: src/app/routes/article/article.controller.ts:30  article.controller.ts; GET /articles
    · e.g. Services: src/app/routes/article/article.service.ts  article.service.ts
    · e.g. Data: src/app/models/http-exception.model.ts  http-exception.model.ts

  Languages
  ─────────
  TypeScript        ██████████████████████░░░░░░   82%      1,861 LOC
  JSON              ███░░░░░░░░░░░░░░░░░░░░░░░░░   14%        331 LOC
  Markdown          ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    2%         48 LOC
  JavaScript        ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0%          2 LOC
  TOML              ░░░░░░░░░░░░░░░░░░░░░░░░░░░░    0%          1 LOC

  Top 5 Files by LOC
  ──────────────────
  1  ./src/app/routes/article/article.service.ts              590 LOC   TypeScript
  2  ./src/tests/services/auth.service.test.ts                183 LOC   TypeScript
  3  ./src/app/routes/auth/auth.service.ts                    159 LOC   TypeScript
  4  ./src/app/routes/article/article.controller.ts           145 LOC   TypeScript
  5  ./src/tests/services/article.service.test.ts             108 LOC   TypeScript

  Dead Code
  ─────────
  medium  jest.preset.js
          Nothing imports it, but "jest.preset" is mentioned in 3 other files
  medium  src/prisma/seed.ts
          Nothing imports it, but "seed" is mentioned in 2 other files
  low     e2e/src/support/global-setup.ts
          A test file, run by the test runner rather than imported
  low     e2e/src/support/global-teardown.ts
          A test file, run by the test runner rather than imported
  low     e2e/src/support/test-setup.ts
          A test file, run by the test runner rather than imported

  Circular Imports
  ────────────────
  src/app/routes/article/article.model.ts → src/app/routes/article/comment.model.ts → src/app/routes/article/article.model.ts
```

</details>

<details>
<summary><b><code>audit --deps</code>: known vulnerabilities in dependencies</b></summary>

<br>

`codedna audit . --deps` (first findings of 39):

```text
  Audit: 67 files, 39 findings, risk 100/100 (high)

  Checked 733 package versions from package-lock.json against osv.dev.

  HIGH    package-lock.json  (high confidence)
          @babel/plugin-transform-modules-systemjs 7.23.3 (npm) has 1 known vulnerability, e.g. "@babel/plugin-transform-modules-systemjs generates arbitrary code when compiling malicious input": GHSA-fv7c-fp4j-7gwp. Fixed in 8.0.0-alpha.13.
  HIGH    package-lock.json  (high confidence)
          axios 1.6.2 (npm) has 31 known vulnerabilities, e.g. "axios Vulnerable to Full Man-in-the-Middle via Prototype Pollution Gadget in `config.proxy`": GHSA-35jp-ww65-95wh, GHSA-3g43-6gmg-66jw, GHSA-43fc-jf86-j433, GHSA-4hjh-wcwx-xvwj, GHSA-6chq-wfr3-2hj9 and 26 more. Fixed in 1.20.0.
  HIGH    package-lock.json  (high confidence)
          body-parser 1.20.1 (npm) has 2 known vulnerabilities, e.g. "body-parser vulnerable to denial of service when url encoding is enabled": GHSA-qwcr-r2fm-qrc7, GHSA-v422-hmwv-36x6. Fixed in 2.3.0.
  HIGH    package-lock.json  (high confidence)
          body-parser 1.20.2 (npm) has 2 known vulnerabilities, e.g. "body-parser vulnerable to denial of service when url encoding is enabled": GHSA-qwcr-r2fm-qrc7, GHSA-v422-hmwv-36x6. Fixed in 2.3.0.
  …
```

Without `--deps` nothing leaves your machine, and this project has no other findings.

</details>

<details>
<summary><b><code>runbook</code>: how to run it</b></summary>

<br>

```text
  Runbook: 4 commands, 4 environment variables, 20 routes

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
    GET     /articles/feed                       src/app/routes/article/article.controller.ts:45
    POST    /articles                            src/app/routes/article/article.controller.ts:71
    GET     /articles/:slug                      src/app/routes/article/article.controller.ts:87
    PUT     /articles/:slug                      src/app/routes/article/article.controller.ts:110
    DELETE  /articles/:slug                      src/app/routes/article/article.controller.ts:129
    GET     /articles/:slug/comments             src/app/routes/article/article.controller.ts:149
    POST    /articles/:slug/comments             src/app/routes/article/article.controller.ts:170
    DELETE  /articles/:slug/comments/:id         src/app/routes/article/article.controller.ts:190
    POST    /articles/:slug/favorite             src/app/routes/article/article.controller.ts:210
    DELETE  /articles/:slug/favorite             src/app/routes/article/article.controller.ts:230
    POST    /users                               src/app/routes/auth/auth.controller.ts:14
    POST    /users/login                         src/app/routes/auth/auth.controller.ts:30
    GET     /user                                src/app/routes/auth/auth.controller.ts:45
    PUT     /user                                src/app/routes/auth/auth.controller.ts:61
    GET     /profiles/:username                  src/app/routes/profile/profile.controller.ts:14
    POST    /profiles/:username/follow           src/app/routes/profile/profile.controller.ts:34
    DELETE  /profiles/:username/follow           src/app/routes/profile/profile.controller.ts:54
    GET     /tags                                src/app/routes/tag/tag.controller.ts:13
    GET     /                                    src/main.ts:21

  Nothing was run. Routes come from common framework patterns and may
  be incomplete.
```

</details>

<details>
<summary><b><code>topology</code>: what will run</b></summary>

<br>

On [Google's microservices demo](https://github.com/GoogleCloudPlatform/microservices-demo):

```text
Kubernetes
  Deployment frontend  (./kubernetes-manifests/frontend.yaml:15)
    image frontend · port 8080
    reached by Service frontend (ClusterIP, port 80→8080) · Service frontend-external (LoadBalancer, port 80→8080)
    code ./src/frontend/ (from skaffold)
  Deployment redis-cart  (./kubernetes-manifests/cartservice.yaml:89)
    image redis:alpine · port 6379
    reached by Service redis-cart (ClusterIP, port 6379)

51 unit(s) in 39 file(s). Read from configuration only; nothing was run or deployed.
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
     5        2       183  src/tests/services/auth.service.test.ts

  Ranked by commits × lines of code. Large files that change often
  are where bugs and refactoring effort usually gather.
```

</details>

<details>
<summary><b>Monorepo workspaces</b></summary>

<br>

Workspaces declared at the repository root are listed with their packages, and each package gets its own numbers. On [tokio](https://github.com/tokio-rs/tokio), `codedna analyze` shows:

```text
  Workspaces
  ──────────
  Cargo (Cargo.toml): 10 packages
    benches                   2.2k LOC · Rust · Tokio · uses tokio, tokio-stream, tokio-util  ./benches
    examples                  1.4k LOC · Rust · Tokio · uses tokio, tokio-stream, tokio-util  ./examples
    stress-test               58 LOC · Rust · Tokio · uses tokio  ./stress-test
    tests-build               257 LOC · Rust · Tokio · uses tokio  ./tests-build
    tests-integration         377 LOC · Rust · Tokio · uses tokio, tokio-test  ./tests-integration
    tokio                     85.4k LOC · Rust · uses tokio-macros, tokio-stream, tokio-test, tokio-util  ./tokio
    tokio-macros              979 LOC · Rust · Tokio · uses tokio  ./tokio-macros
    tokio-stream              5.7k LOC · Rust · Tokio · uses tokio, tokio-test, tokio-util  ./tokio-stream
    tokio-test                1.3k LOC · Rust · Tokio · uses tokio, tokio-stream  ./tokio-test
    tokio-util                13.5k LOC · Rust · Tokio · uses tokio, tokio-stream, tokio-test  ./tokio-util
```

`codedna packages` gives the detail for each package, including who uses it:

```text
  tokio-util  ./tokio-util
    13,534 LOC  ·  94 files  ·  Rust
    Stack:    Tokio
    Uses:     tokio, tokio-stream, tokio-test
    Used by:  benches, examples, tokio, tokio-stream
```

A file belongs to the deepest package folder that holds it. "Uses" comes from the package's manifest dependencies on other packages in the repository (`package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`) and from imports that cross package folders. Languages leave out JSON, Markdown, TOML and YAML unless a package has no code.

</details>

<details>
<summary><b>JSON output</b></summary>

<br>

`codedna json .` on the same project (`dependency_graph`, `dependencies`, `file_breakdown` and `architecture_details` shortened):

```json
{
  "schema_version": 1,
  "analyzer": {
    "name": "codedna",
    "version": "0.2.11",
    "confidence": [
      { "kind": "dead_code", "level": "high", "status": "insufficient_data", "set": "third", "outcomes": 9, "repositories": 6 },
      { "kind": "audit", "level": "high", "status": "insufficient_data", "set": "third", "outcomes": 13, "repositories": 8 },
      { "kind": "architecture", "level": "high", "status": "insufficient_data", "set": "third", "outcomes": 20, "repositories": 20 }
    ]
  },
  "project_type": "Backend API service",
  "total_loc": 2243,
  "comment_lines": 245,
  "blank_lines": 294,
  "languages": {
    "Markdown": 48,
    "TOML": 1,
    "TypeScript": 1861,
    "JSON": 331,
    "JavaScript": 2
  },
  "frameworks": ["Express"],
  "databases": ["Prisma"],
  "infrastructure": ["Docker"],
  "architecture": "API → Database",
  "architecture_details": {
    "layers": [{
      "name": "Routes", "files": 11, "confirmed_by_code": 5, "placed_by_imports": 0, "dirs": ["./src/app/routes/auth/", "..."],
      "examples": [{ "file": "src/app/routes/article/article.controller.ts", "placed_by": "name", "matched": "article.controller.ts", "code": { "line": 30, "sign": "GET /articles" } }]
    }],
    "flows": [{
      "from": "Services", "to": "Data", "imports": 12,
      "examples": [{ "from": "src/app/routes/article/article.service.ts", "to": "src/prisma/prisma-client.ts" }]
    }],
    "confidence": "high",
    "evidence": ["Services → Data: 12 imports"]
  },
  "dead_code": [
    "./e2e/src/support/global-setup.ts",
    "./e2e/src/support/global-teardown.ts",
    "./e2e/src/support/test-setup.ts",
    "./jest.preset.js",
    "./src/prisma/seed.ts"
  ],
  "dead_code_details": [
    {
      "file": "./jest.preset.js", "confidence": "medium", "reason": "Nothing imports it, but \"jest.preset\" is mentioned in 3 other files",
      "evidence": [{ "file": "e2e/jest.config.ts", "line": 4, "text": "preset: '../jest.preset.js'," }]
    }
  ],
  "circular_imports": [
    ["./src/app/routes/article/article.model.ts", "./src/app/routes/article/comment.model.ts"]
  ],
  "workspaces": [],
  "dependency_graph": { "...": ["..."] },
  "dependencies": {
    "rules_version": 1,
    "facts": [
      {
        "source": "./src/app/app.config.ts",
        "specifier": "./app.routes",
        "targets": ["./src/app/app.routes.ts"],
        "kind": "runtime_import",
        "resolution": "resolved",
        "certainty": "direct",
        "evidence": { "file": "./src/app/app.config.ts", "line": 4, "span": [188, 200], "rule": "js.import" }
      }
    ]
  },
  "file_breakdown": [
    { "file": "./src/app/routes/article/article.service.ts", "loc": 590, "language": "TypeScript" }
  ]
}
```

The report is versioned. `schema_version` goes up only when a field is removed, renamed or changes meaning; new fields can appear without it changing, so ignore fields you do not know. The full shape is in [`schema/report.schema.json`](schema/report.schema.json) (also `codedna schema`). Reports saved before versioning have no `schema_version` and read as 0.

| Field | Type | Description |
|---|---|---|
| `schema_version` | number | Version of the report shape (currently 1) |
| `analyzer` | object | `name` and `version` of the CodeDna build that wrote it, and `confidence`: for each kind, whether its `high` level passed the release gate on the sealed set (`validated`, `unvalidated` or `insufficient_data`) and on how many labels |
| `project_type` | string | High-level project classification |
| `total_loc` | number | Lines of code, excluding comments, blank lines and lockfiles |
| `comment_lines` | number | Comment-only lines across detected source files |
| `blank_lines` | number | Blank lines across detected source files |
| `languages` | object | Language → lines of code |
| `frameworks` | string[] | Detected frameworks |
| `databases` | string[] | Detected databases and ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Architecture summary from detected frameworks and databases |
| `architecture_details` | object | Layers found in the code (`layers`, each with `confirmed_by_code`, `placed_by_imports`, up to 3 `examples` and `also_shown`: how many of its files also show another layer), import counts between them (`flows`, each with up to 3 example imports), `confidence` (`high`/`medium`/`low`), `evidence`, `packages` (each `{ path, purpose, signals, files, layers }`, `purpose` one of `application`, `library`, `cli`, `template`, `unknown`), `competing` (readings the layers leave out) and `unknown` (set when most source is in languages CodeDna does not read) |
| `dead_code` | string[] | Files nothing reaches from an entry point |
| `dead_code_details` | object[] | Per file `{ file, confidence, reason, evidence, status, conditions }`: how likely it is to be really unused, why, the lines behind the reason (`{ file, line, text }`, up to 3), and `status`: `unreachable` (nothing analyzed could load it), `possible` (a possibility, such as a computed `import()` whose folder matches, may load it) or `unknown` (an unresolved import may name it); `conditions` lists what may load it |
| `usage` | object | How files are used, judged from confirmed entry points: `summary` counts `confirmed` (reached through runtime imports), `possible` (only through an ambiguous or lazy import), `type_only` (only through type imports, or a declaration file), `test_only`, `tooling_only` (configs, stories, task runners) and `unreachable`; `files` lists every reached file that is not confirmed as `{ file, usage, path }`, where `path` runs from the entry point to the file |
| `circular_imports` | string[][] | Groups of files that import each other, directly or through other files |
| `cycle_details` | object[] | The same cycles in the same order, as `{ files, kind }`: `kind` is `runtime` (closes through runtime imports only), `possible` (needs an ambiguous import) or `type_only` (needs a type import, so it is no loop at runtime) |
| `workspaces` | object[] | Monorepo workspaces declared at the root: `{ tool, manifest, packages: [{ name, path, files, loc, languages, frameworks, databases, dead_files, depends_on, used_by }] }` |
| `dependency_graph` | object | File → imported files. A projection of `dependencies`: the targets of its resolved facts |
| `dependencies` | object | Typed dependency facts: `rules_version` and `facts`, one per dependency found, sorted by source and position. Each has `source`, `specifier` (as written), `targets` (files it resolved to), `kind` (`runtime_import`, `type_only_import`, `re_export`, `worker_entry`, `process_entry`, `resource_reference`, `configured_entry`, `possible_dynamic_load`), `resolution` (`resolved`, `external`, `unresolved`, `ambiguous`, `unsupported`), `certainty` (`direct`, `declaration`, `inferred`, `possibility`), an optional `condition` (`dynamic` for `import()`; for a package import, the condition a target applies under, such as `types` or `browser`, several joined with `+`) and `evidence` (`file`, `line`, byte `span` where known, `rule` id such as `js.import` or `python.from_import`). Unresolved and external imports are kept. `entries` holds the `configured_entry` facts of framework adapters (source: the package's `package.json`; `evidence.rule` such as `next.app_router@v1`), and `adapters` lists each adapter active in a package: `package`, `adapter`, `version`, the `manifest` and `line` of the dependency that activated it, and its `config` files |
| `coverage` | object | How much the dependency analysis covered: `source_files`, `analyzed_files` (imports followed), `unreadable_files`, `not_analyzed` (languages whose imports are not followed, `{ language, files }`), `imports` (`{ internal, resolved, unresolved, external, ambiguous, inferred }`; `inferred` counts resolved imports that rest on a guess), `unresolved` (up to 20 `{ file, line, import }`), `parse` (JS/TS files `parsed`, `partial` and `failed`, and up to 20 `failures` as `{ file, line, message, status }`) and a one-line `summary`. Read it before trusting an empty `dead_code` or `circular_imports` |
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

**Languages:** Rust, TypeScript, JavaScript, Python, Go, Java, Kotlin, C#, Ruby, PHP, Swift, C, C++, Scala, Dart, Vue, Svelte, Solidity, CSS, HTML, TOML, JSON, YAML, Markdown, Shell. Imports are followed for TypeScript, JavaScript, Python and Rust; other languages are counted and sized, and `coverage` names them as not analyzed.

| Ecosystem | Frameworks |
|---|---|
| **JS / TS** | React, Next.js, Vue, Nuxt, Svelte, Astro, Remix, Gatsby, Express, Vite, NestJS, Fastify, Koa, Hapi |
| **Python** | FastAPI, Django, Flask, Starlette, Tornado, aiohttp, Pyramid, Sanic, Litestar |
| **Rust** | Tokio, Axum, Actix-web, Rocket, Warp, Tonic, Poem, Salvo, Tide, Hyper, Tauri, Leptos, Dioxus, Yew |
| **Go** | Gin, Echo, Fiber, Gorilla Mux, Beego, Chi, Revel, Uber FX |

| Manifest | Databases and ORMs |
|---|---|
| **`package.json`** | PostgreSQL, MongoDB, MySQL, Redis, Prisma, SQLite, Knex, Sequelize, TypeORM, Drizzle ORM, Cassandra, CouchDB, Elasticsearch, DynamoDB |
| **`requirements.txt` / `pyproject.toml`** | SQLAlchemy, SQLModel, PostgreSQL, MongoDB, Redis, MySQL, Tortoise ORM, Peewee, Databases, Elasticsearch, Cassandra |
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

Paths matched by the repository's `.gitignore` files (root and nested) and `.ignore` files are skipped. `node_modules/` and `.git/` are always skipped, as are files over 10 MiB and symbolic links. `target/`, `dist/`, `build/` and `coverage/` are skipped where they hold build output: at the top of the scan, next to a project manifest (`package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`, …), or where the enclosing package declares them as output (`package.json` `main`/`module`/`types`/`browser`/`bin`/`exports`/`files`, `tsconfig.json` `outDir`). Elsewhere they are source folders and are scanned, such as `packages/astro/src/core/build`.

</details>

<details>
<summary><b>Accuracy notes</b></summary>

<br>

- **Architecture** comes from two sources. The summary line uses detected frameworks and databases; manifests in `examples/`, `demo/`, `docs/` and test folders count only when the repository has no others, so a library's sample apps don't make it read as a web service. `architecture_details` then checks every source file, tests left out. A file is placed in a layer by its code: route handlers (Express, Nest, FastAPI, Django, Go, Spring, ASP.NET, Rails, Laravel, Rust), middleware signatures, ORM models and tables, and service annotations. When the code shows nothing, its folder or file name places it (`routes/`, `user.controller.ts`, `services/`, `models.py`, …). Code wins over a name that disagrees (tables kept in `services/` are Data), and the example records the name it overruled (`name_said`: `services/ (Services)`); the one exception is a handler in a routes folder that has the `(req, res, next)` middleware signature, which stays in Routes. A file with no other signal that routes import and that imports data is placed in Services. Imports between layers are then counted. Files in example, demo and docs folders are left out too, unless nothing else has layers. Confidence is high when layers import each other top-down (at least three imports down for each one up), or when every layer's name is backed by its code; medium when layers exist without that support; and low when none are found. Each layer reports `confirmed_by_code` and `placed_by_imports`, and shows up to 3 example files. An example says how it was placed (`placed_by`: `settings`, `name`, `code` or `imports`), what matched (the folder or file name, the `.codedna.toml` entry, or the files it imports and is imported by) and, when the code agrees, the line and sign (`code`: `{ line, sign }`, such as a route `GET /users` or `@Entity(`; line 0 means the whole file, as for a `.vue` component). Code-backed examples come first.
- **Package purpose comes first.** Each manifest (`package.json`, `pyproject.toml`/`setup.py`, `go.mod`, `Cargo.toml`, `Gemfile`, `composer.json`, `cookiecutter.json`) is read to decide whether its package is an `application` (web framework or ORM dependency, `manage.py`, a server `wsgi.py`, a Go `main` with a router, a browser framework with a dev server and an `index.html`), a `cli` (published with a `bin`, console scripts), a `library` (published for import), a `template` or `unknown`. Folder names are only supporting evidence. Files in a cli package never count as layers. Files in a library count only when the repository also has an application that the library serves; a library named `sdk`, `client`, `types` or `typings` never counts. An application inside a repository whose root is a library is treated as a demo. What is left out is reported in `competing` (`packages/vite is a cli …: names or code suggest Middleware (14 files); not counted as application layers`), so the other reading stays visible. Test harnesses, mocks, stories, benchmarks, playgrounds and codemods are never layers. When product source in unread languages (Elixir, Swift, C, …) outnumbers source CodeDna reads, no layers are reported and `unknown` says why. A file can show several roles: it is placed in one layer and `also_shown` counts the others its code shows (a Django `models.py` that also defines views).
- **Unusual layouts** can name their layers in a `.codedna.toml` at the repository root. Keys are `ui`, `routes`, `middleware`, `services` and `data`; values are path prefixes. The longest prefix wins, and settings win over names and code:

  ```toml
  [layers]
  routes = ["src/http/"]
  data = ["src/store/", "src/tables.rs"]
  ```
- **Lines of code** count only lines that contain code. Comment-only and blank lines are reported separately (`comment_lines`, `blank_lines`), and comment markers inside strings are not treated as comments. Lockfiles (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock` and similar) are left out so they don't swamp the totals.
- **Dead code is a heuristic, with a confidence for each file.** A file is listed when nothing reaches it through imports from an entry point. Runtime, type, test and tooling usage are judged apart: a file reached only through `import type`, only from tests or only from configs is not dead code, but `usage` says it is not confirmed as used by the program, with the path from the entry point. An unresolved import or a possibility that could name a candidate makes it `unknown` or `possible` with the condition listed; one that could name nothing in the candidate's folder changes nothing. CodeDna never calls a file safe to delete. Each one is then rated with a reason. *High*: its name appears nowhere else in the repository. *Medium*: nothing imports it, but its name is mentioned elsewhere or the repository loads modules dynamically. *Low*: it is probably loaded another way, for example from a convention folder (`migrations/`, `alembic/`, `management/`, `scripts/`, …), as a test, as a Django app module (`models.py`, `admin.py`, `signals.py`, …), by a dynamic `import()`/`require()`/`importlib` call that names it, or by a path built from a template string (`` `index-drizzle/with-${db}.ts` ``) that matches its folder and name. Python dotted strings in settings (`"app.views"`) count as imports. When the reason rests on lines elsewhere, such as files that mention the name or the dynamic loader, `evidence` gives up to 3 of them with file, line and text. Check before deleting anything, even at high confidence.
- **Framework adapters name the files a framework loads without an import.** Each package is judged by its own `package.json`: an adapter is active only where that manifest depends on the framework (or, for a test runner, has its config). A dependency in an example or dev-only package activates nothing at the repository root, and a folder that only looks like `app/` or `pages/` is not a route root. An active adapter adds `configured_entry` facts to `dependencies.entries`, and dead-code reachability starts from those files.

  | Adapter | Active when the package has | Files it names | Rules |
  | --- | --- | --- | --- |
  | `node` | `main`, `module`, `browser`, `bin`, `exports` or `scripts` | Files those fields name (a single `*` in `exports` matches package files; `null` subpaths and targets outside the package are refused), and source files a script or `prisma.seed` runs | `node.package_entry@v1` (declaration), `node.script@v1` |
  | `next` | `next` | App Router files in `app/` (`page`, `layout`, `route`, `loading`, `error`, `not-found`, metadata files, …) including route groups, skipping `_private` folders; every file in `pages/`; `middleware`, `instrumentation` and `proxy` at the root or in `src/`; `next.config.*`. Root `app/`/`pages/` win over `src/`, and literal `pageExtensions` are honoured | `next.app_router@v1`, `next.pages_router@v1`, `next.special_file@v1`, `next.config@v1` |
  | `expo_router` | `expo-router` | Every file in the routes folder: the `expo-router` plugin's `root` in `app.json`, else `app/`, else `src/app/` | `expo.router@v1` |
  | `vitest` | `vitest` or `vitest.config.*` | `include` globs (default `*.test.*`, `*.spec.*`, type tests `*.test-d.*`, and `*.bench.*`), `setupFiles`, `globalSetup`; the `test` block of `vite.config.*` when there is no Vitest config | `test.vitest@v1`, `test.setup@v1` |
  | `jest` | `jest`, `jest.config.*` or a `"jest"` key | `testMatch` (default `__tests__/`, `*.test.*`, `*.spec.*`), setup and global setup/teardown files | `test.jest@v1`, `test.setup@v1` |
  | `playwright` | `@playwright/test` or `playwright.config.*` | `testMatch` under `testDir`, global setup/teardown | `test.playwright@v1`, `test.setup@v1` |
  | `mocha` | `mocha`, `.mocharc.*` or a `"mocha"` key | `spec` (default `test/*.{js,cjs,mjs}`), `require` and `file` | `test.mocha@v1`, `test.setup@v1` |
  | `tsd` | `tsd` | `index.test-d.ts`, `test-d/**/*.ts` | `test.tsd@v1` |
  | (any) | A dependency with a `__mocks__/<name>` file next to its `package.json` | That manual mock | `test.module_mock@v1` |

  Test-runner facts carry the condition `test`. Every adapter is version 1; a fact's rule names its version. Where the `next` adapter is active, Next.js file names (`page`, `layout`, …) and `pages/` no longer make a file live by themselves; elsewhere the old `pages/` folder convention still applies (Astro, Gatsby, Docusaurus). Limitations: only literal config values are read (a computed `include` or `testRegex` falls back to nothing, not to defaults); a runner config at the repository root does not reach files owned by a nested `package.json` (Vitest `projects`, Jest `projects`); and file-name conventions of other frameworks (Remix, SvelteKit, Nuxt, Astro) remain heuristics, not adapters.
- **JavaScript and TypeScript imports come from a syntax tree.** Each `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts` and `.cts` file is parsed, and so are the `<script>` blocks of `.vue` and `.svelte` files and the frontmatter and `<script>` blocks of `.astro` files. Markup outside them is not read. What counts:

  | Syntax | Fact | Rule |
  | --- | --- | --- |
  | `import x from "./a"`, `import * as ns`, `import { a as b }`, over several lines | runtime import | `js.import` |
  | `import type { T }`, `import { type T }` (only type names) | type-only import | `js.import_type` |
  | `import "./side"` | runtime import | `js.side_effect_import` |
  | `export { a } from "./a"`, `export * from "./a"` | re-export | `js.export_from` |
  | `export type { T } from "./t"` | type-only import | `js.export_type_from` |
  | `type T = import("./t").T` | type-only import | `js.import_type_expression` |
  | `/** @import { T } from "./t" */`, `/** @type {import("./t").T} */` in a JavaScript file, kept only when it names a package or another existing file (`./t` may be `t.d.ts`) | type-only import | `js.jsdoc_import` |
  | `import fs = require("./fs")` (TypeScript) | runtime or type-only import | `js.import_equals` |
  | `require("./c")` with the global `require` | runtime import | `js.require` |
  | `import("./lazy")` | runtime import, condition `dynamic` | `js.dynamic_import` |
  | `` import(`./pages/${name}`) ``, `import(path)`, `require(name)` | possibility | `js.template_import`, `js.computed_import`, `js.computed_require` |
  | `require()` when the file declares its own `require` | possibility | `js.require_shadowed` |
  | `new URL("./w.js", import.meta.url)`, `new Worker(...)`, process loaders | worker, resource or process entry | `js.new_url`, `js.worker`, `js.process` |
| `import.meta.glob("./routes/*.ts")`, `import.meta.glob([...])`, `globEager` | runtime import (resource with `query`/`as`) | `js.import_meta_glob` |
| `` import.meta.glob(`./${dir}/*.ts`) ``, `import.meta.glob(pattern)`, a computed `base` | possibility | `js.import_meta_glob_computed` |

  A specifier made only of string literals (`import("./pages" + "/home")`, or a template with no `${}`) is folded and counts like a written one. Anything else, including a variable that holds a constant string, is a possibility. Possibilities are listed in `dependencies` but add no edge to the graph, the dead-code reachability or the cycle check. A file with a syntax error the parser can recover from is reported as `partial` and keeps the facts from the recovered tree. A file the parser gives up on is reported as `failed`; a text scan then lists the quoted `import`/`require` specifiers it finds, as possibilities only. Both show in `coverage.parse` with the first error's line. A literal `import.meta.glob` pattern is expanded against the scanned files the way Vite does: `./` and `../` patterns from the importing file, `/` patterns from the Vite root (the nearest folder with a `vite.config.*` inside the package, else the package folder), a literal `base` moved into the relative patterns, `!` patterns removed, `node_modules` and dot folders skipped unless named, and never the importing file itself. Each match is a target of the one fact (no match is `unresolved`; alias and package patterns such as `@/x/*` are not expanded and, like more than 10,000 matches, are an `unsupported` possibility with no edge). Limitations: a `.js` file that does not parse as JavaScript is retried as TypeScript with JSX, which reads JSX and most Flow annotations but not all Flow syntax; bundler globs other than Vite's `import.meta.glob` (webpack `require.context`) are not followed, and markup references in components (`<img src>`, `<link href>`) are not dependencies.
- **Files loaded by name count only when code loads them.** Besides imports, a JS/TS file is reached when it is the first argument of `new URL(..., import.meta.url)` or `new Worker(...)`/`new SharedWorker(...)` (when the file defines or imports its own `Worker`, the call is only a possibility), or is named among the arguments of `spawn`, `fork`, `exec`, `execFile` and their `Sync` forms imported from `child_process`, of `execa` or `cross-spawn`, or of `Bun.spawn`. The file must be named with its extension and is looked up next to the calling file. A path in a log message, comment or any other string, or passed to a local function that shares a loader's name, is not a dependency. Paths built at runtime and loaders reached through other wrappers are not followed.
- **Code signs count only in code.** Words such as `@RestController`, `pgTable(` or `@app.get(` inside a string, a comment or a test fixture are not taken as evidence for a layer or a route.
- **JavaScript and TypeScript imports resolve like Node.js and `tsc`, within the repository.** The resolver (`src/resolution/javascript.rs`) follows Node's ESM resolution algorithm (`PACKAGE_RESOLVE`, `PACKAGE_EXPORTS_RESOLVE`, `PACKAGE_IMPORTS_RESOLVE`) and TypeScript's module resolution, and is checked against `tsc --traceResolution` (TypeScript 5.9) and Node.js 22 on pinned fixtures. It is not a full reimplementation of either:

  | Input | What CodeDna does |
  |---|---|
  | Relative paths | Extension and index lookup; `./x.js` finds `x.ts`, `x.tsx`, `x.d.ts` (TypeScript extension substitution). When both `x.js` and `x.ts` exist, `x.js` is what runs and `x.ts` is kept as the `types` target |
  | Package scope | The nearest `package.json` sets the package and its module mode (`type`, `.mjs`/`.cjs`/`.mts`/`.cts`), which picks the `import` or `require` condition for `import`/`require()` |
  | `exports` / `imports` (`#x`) | Plain, conditional, nested and `*` patterns, `null` exclusions, self-reference by the package's own name. `default`, `import`/`require` and the project's `customConditions` are decided as Node does; any other condition (`browser`, `node`, `development`, `workerd`, `types` …) is not chosen: every branch is kept, labelled with its `condition` |
  | `main`, `module`, `types`, `source` | Used when there is no `exports`, then `index` |
  | `tsconfig.json` / `jsconfig.json` | The config that includes the file (nearest, or one listed in `references`), with `extends` (relative, package and arrays), `paths`, `baseUrl`, `${configDir}`, `moduleResolution`, `customConditions`, `outDir`/`rootDir` |
  | Workspace packages | Packages of npm, Yarn, Bun and pnpm workspaces resolve by name. Same-named packages in one workspace are `ambiguous` with every candidate; packages of a separate workspace (example monorepos) do not compete |
  | Built output | A target in `dist/` (or another build folder) that is not in the repository maps back to its source: `direct` when `outDir`/`rootDir` say so, `inferred` when guessed from folder names |
  | Bare packages that are not the repository's own | `external`; there is no `node_modules` lookup |
  | Aliases in `vite.config.*`, `webpack.config.*`, `jest.config.*` and other executable configs | Not run. The config gets an `unsupported` fact (`js.config_alias_unsupported`), and imports through its aliases may stay unresolved |

  Resolution is fenced to the scanned files: a target must be a file the scanner listed, and only listed config files are read (at most 1 MiB each). Folders the scanner skips (`node_modules`, and `dist`/`build`/`target`/`coverage` where they hold build output) are never targets. A dependency with several targets adds an edge to each, so a file used only by one condition stays reachable.
- **Coverage** says how much of the code the dependency graph covers, so an empty dead-code or cycle list can be told apart from code that was never looked at. It counts the source files whose imports were followed, imports of the repository's own code that matched no file (relative paths, path aliases, workspace packages, the project's own Python packages and `mod` declarations), with file and line for the first 20, and languages whose imports are not followed. The terminal report, `context`, reports and MCP tools show it; `deadcode` and `cycles` say "not checked" when no file could be analyzed.
- **Audit** matches known risky patterns. It cannot prove code is safe: a clean result only means none of its checks matched, and `checks` in the JSON lists each check, whether it ran and what it covers. Each finding keeps three things apart: a **severity** (how bad it would be if real), an **evidence strength** (`strong`, `moderate` or `weak`: what the rule actually saw, with `signals` saying why) and a **confidence** (how likely it is real, not a false alarm). Per family:
  - **Secrets** need a documented key format; entropy alone is never enough. Published example tokens, values cut short with `...`, counting filler such as `1a2b3c` and keys a nearby comment calls fake or throwaway are weak. A key's folder explains it but does not excuse it: a key under `tests/` or `src/test/resources` that the project's own code reads keeps its confidence, one used only by tests or by nothing is low, and a key in docs is lowered one step. Only references that resolve to the key's file count (its path, its classpath name, a relative JavaScript or Python import); a file of the same name elsewhere or a config file mounting it does not, and aliased imports (`@/lib/keys`) are not followed.
  - **Obfuscation** is strong only when a decoder sits inside the arguments of `eval`, `exec` or `Function`; a decoder and a code runner merely on the same line is weak, and hex escapes in a large generated bundle are weak. Nothing is decoded or run, and the argument search is bounded.
  - **Hidden Unicode**: bidirectional controls (Trojan Source) stay high; an invisible character inside a string literal or comment is weak, in a name it is not.
  - **Install scripts** say which lifecycle hook runs and which host a script downloads from. A download piped to a shell is strong; an npm `prepare` script (registry installs do not run it) and a prebuilt platform-binary download are weak.
  - **Dependencies** (`--deps`) give the package, version, advisories and fixed version of each finding, and the summary gives the database, when it was queried (`checked_at`) and a `status` (`checked`, `no-lockfile` or `error`). Whether the vulnerable code is reachable is not checked.

  Findings in tests, docs, examples, fixtures, vendored libraries (`vendor/`, `.yarn/`, `*.min.js`, `*-bundle.js`) and translations (`locales/`, `i18n/`) are lowered one step and marked with their `context`. Long lines, encoded data and weak obfuscation or Unicode findings in tests, fixtures, vendored libraries and translations are not reported.
- **Monorepos**: workspaces declared at the repository root are listed with their packages (`workspaces` in JSON): npm, Yarn and Bun workspaces, pnpm (`pnpm-workspace.yaml`), Lerna, Cargo, Go (`go.work`) and uv. Each package gets its own size, languages, stack, dead files and links to the other packages (`codedna packages`).
- **Routes** in `runbook` come from common framework patterns and may be incomplete.
- **Topology** reads deployment files as written; nothing is rendered or deployed. Helm values are not applied, so templated names stay as `{{ … }}`, and Kustomize overlays are listed as files, not merged. A Service links to the workloads its selector matches, preferring ones in the same file, then the same folder. Code is found from Skaffold and Compose build contexts, an image named like a folder with a Dockerfile, or a function's handler. Files in test, example and docs folders are skipped and counted.
- **Change impact** follows the same imports as the dependency graph, backwards from the changed files, so a file reached another way (configuration, reflection, a language whose imports are not followed) is not listed. Each affected file has a `kind`, by the strongest chain that reaches it: `runtime`, `possible` (through an ambiguous import) or `type_only` (only through type imports; its runtime behaviour does not change through that chain). `unresolved` lists imports that did not resolve but could name a changed file (`{ file, line, specifier, may_name }`, up to 50): the impact may reach further than listed. Untracked files count as added. Tests are files that look like tests (`*.test.ts`, `test_*.py`, `tests/`, …) and import changed code, directly or through other files.

</details>

## 📏 Measured confidence

Confidence levels are measured on hand-checked results, not guessed. Precision is how many results at a level were right, with a 95% interval.

**Third set (the honest figure).** 43 more repositories, new to CodeDna and pinned to a commit: apps, monorepos, libraries, CLIs and deliberately vulnerable apps in TypeScript, Python, Go, Rust, PHP, Ruby and Java. This release was run on them before it shipped and 319 results were checked by hand, sampled per repository: up to 5 dead-code files and 5 audit findings across the levels, and each architecture. No detector was changed after these labels were read.

| Result | High | Medium | Low |
|---|---|---|---|
| Dead code | 67% (6/9) | 42% (39/92) | 9% (7/82) |
| Audit | 15% (2/13) | 0% (0/24) | 2% (1/56) |
| Architecture | 45% (9/20) | 11% (2/18) | 80% (4/5) |

- **Dead code** is about where the fresh set put it: a lead at every level. The misses are again files loaded without an import: `go:embed`, esbuild and Vite entry points, Nuxt middleware, editor plugin folders, docs pages that include a file, scripts run by Docker, CI or `tox`, and type tests checked through a `tsconfig`.
- **Audit** findings outside the clearly real ones were test keys, placeholder tokens, generated or vendored files, intentional Unicode and fixture data.
- **Architecture is weaker than the fresh set suggested.** Libraries and CLIs were described as apps ("Frontend only", "API only"), a Rails app was reported as Python, and some layers came from `vendor/` or test mocks. Treat the summary as a starting point.

**Fresh set.** 57 repositories that had never been run through CodeDna before (web apps, monorepos, libraries, starters and deliberately vulnerable apps, each pinned to a commit). v0.2.8 was run on them and 361 results were checked by hand: every high-confidence dead-code file (up to 10 per repository), a sample of medium and low ones, up to 5 audit findings per level, and each architecture. v0.2.10 then fixed detectors after reading these labels (mocks of dependencies, vendored code, translations and docs in the audit), so these figures, measured with v0.2.8, are no longer independent.

| Result | High | Medium | Low |
|---|---|---|---|
| Dead code | 50% (25/50) | 47% (35/75) | 5% (4/85) |
| Audit | 30% (3/10) | 3% (1/30) | 0% (0/52) |
| Architecture | 88% (28/32) | 76% (16/21) | 100% (3/3) |

- **Dead code is a lead at every level, including high.** Most wrong ones are files a tool or framework loads without an import: by file-name convention (Expo and Next.js special files, Alembic and TypeORM migrations, test setup and mocks, type tests), by folder scan (a computed `import.meta.glob`, webpack `require.context`, plugin and migration folders), by a script in `package.json` or CI, or by a module name in settings. A few were imported by a relative path CodeDna missed.
- **Audit findings are mostly noise outside high.** Most wrong ones were vendored or minified code (such as Yarn's own `.yarn/releases` bundle), test fixtures and translation strings.
- **Architecture** is the most reliable: when CodeDna names layers at high confidence it was right 28 times out of 32.
- One bug found while labelling was fixed in v0.2.9: a Django app without an `apps.py` was reported as having a dead `apps.py`. The numbers above are for v0.2.8, before that fix.

**Original set.** 31 other repositories, labelled with v0.2.6 (301 results): dead code high 59% (29/49), medium 54% (26/48), low 12% (12/103); audit 2/4, 9/22, 2/33; architecture 16/17, 2/10, 4/4. v0.2.7 fixed detectors after reading these labels, which raised high-confidence dead code on that set to 97% (29/30). That figure is not independent, and the fresh set shows it does not carry over to new repositories.

`codedna calibration` prints all three sets with their intervals (`--set fresh` for one). The repositories and labels are in [`calibration/`](calibration/); clone a set and run `codedna calibration --set fresh --corpus DIR` to score your own build on the same labels.

**How the sets are used.** [`calibration/protocol.json`](calibration/protocol.json) gives each set a role and records every time its labels were read:

- **development** (original, fresh): read in detail, and detectors are changed after reading them;
- **calibration**: used only to set confidence thresholds (none yet);
- **sealed** (third): measured once per release with `--release`, which prints aggregates only. A whole-corpus run skips it.

A level is called *validated* only when `high` reaches 95% precision with both the 95% interval and a bootstrap over repositories above 90%, on at least 200 labels from 20 repositories. No level has that many labels yet, so every `high` reads "insufficient data", and each JSON report says so in `analyzer.confidence`.

A measured run also gives, per kind: precision for each language (dead code) or check (audit), false positives per 1,000 files, selective precision (keeping only results at or above a level, and how many it abstains on), estimated recall once files without a finding have been sampled and labelled, and the macro average over repositories.

- `--fixtures` scores the fully annotated fixtures in [`calibration/fixtures.txt`](calibration/fixtures.txt): exact dead-code recall and precision, the audit's positive controls, and whether a cached run agrees with an uncached one. `--check` fails when a gate fails.
- `--baseline FILE` fails when a labelled result got worse than in a recorded run: a correct one dropped a level, a wrong one rose, or an architecture output changed. `--write-baseline FILE` records one. CI checks the development sets against [`calibration/baseline/development.json`](calibration/baseline/development.json).
- `--queue FILE` writes a random sample of unlabelled results, including files with no finding, to label next.

## ⚡ Performance

`codedna analyze` with v0.1.3 on a 2-core Linux machine, best of 5 runs:

| Repository | Files | Lines of code | Time |
|---|---:|---:|---:|
| [tokio](https://github.com/tokio-rs/tokio) | 886 | 114k | **60 ms** |
| [fastapi](https://github.com/fastapi/fastapi) | 2,993 | 255k | **186 ms** |
| [django](https://github.com/django/django) | 5,699 | 468k | **631 ms** |

Lines of code exclude comments and blank lines.

**Repeat runs are incremental.** CodeDna remembers what each file's content showed (line counts, JavaScript syntax scan, imports, layer, dead-code mentions) and redoes only the work whose inputs changed. On a 2-core Linux machine with v0.2.5:

| Repository | Files | First run | Repeat run |
|---|---:|---:|---:|
| [vscode](https://github.com/microsoft/vscode) | 20,744 | 11.6 s | **1.4 s** |
| [kubernetes](https://github.com/kubernetes/kubernetes) | 31,461 | 4.9 s | **0.9 s** |

The output is identical either way. Every cached value is keyed by a hash of the file's bytes and by the version of the code that produced it, so an edit that keeps the size and modification time is still seen, and a new parser, resolver or rule set recomputes only what it affects. A file's imports are also keyed by everything their resolution looked at: which files exist in the folders it searched, the `package.json` and `tsconfig.json` chain that applies, workspace members and framework config files. Editing a `tsconfig.json` re-resolves only the files that config governs; adding a file re-resolves only the imports that searched its folder. `CODEDNA_CACHE_TRUST_MTIME=1` is a faster mode that trusts an unchanged size and modification time instead of reading every file, and `CODEDNA_CACHE_STATS=1` prints how many values were reused and recomputed. The cache is kept in your user cache folder (`~/.cache/codedna`, `~/Library/Caches/codedna` on macOS, `%LOCALAPPDATA%\codedna\cache` on Windows; `CODEDNA_CACHE_DIR` moves it), never in the repository, and remote clones are never cached. `--no-cache` or `CODEDNA_NO_CACHE=1` reads every file afresh.

## 🛡️ Security

CodeDna is built to be safe to run on any repository, including ones you do not trust.

| | |
|---|---|
| 🔒 **Read-only** | Never runs scripts, build steps, hooks or package managers from the repository. Git history is read without checking anything out. |
| 📴 **Nothing leaves your machine** | No telemetry or update checks. The only network access is cloning a URL you pass, and `audit --deps`, which sends lockfile package names and versions (nothing else) to osv.dev. |
| 🤫 **Secrets stay secret** | `.env` files are never opened. `runbook` reports variable names only, `audit` names the kind of credential and the line but never the value, and tokens in clone URLs are masked. |
| 🗄️ **The cache can't be planted** | Results are cached in your own cache folder (readable only by you), never read from the repository, so a repository cannot ship results for its own files. |
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
├── dependency_graph.rs     import / require / mod parsing into facts
├── parsers/javascript.rs   JS/TS syntax-tree import scan (Oxc)
├── resolution/javascript.rs JS/TS module resolution (package.json, tsconfig, workspaces)
├── resolution/deps.rs      records what each resolution looked at, for the cache
├── facts.rs                typed dependency facts
├── graph.rs                dependency graph built from facts
├── dead_code_detector.rs   reachability from entry points
├── context.rs              token-budgeted briefing for agents
├── mcp.rs                  MCP server (JSON-RPC over stdio)
├── hotspots.rs             churn × size ranking, co-change coupling
├── audit/                  supply-chain red-flag scan: rules/ (one module per family), context, usage
├── baseline.rs             accepted findings, so only new ones are reported
├── cache.rs                per-file results reused on repeat runs
├── vulns.rs                lockfile versions checked against OSV
├── runbook.rs              commands, env var names, API routes
├── topology.rs             deployment files: what runs, how it is reached, which code
├── yaml.rs                 bounded YAML reader for untrusted files
├── diff.rs                 compare two snapshots or versions
├── impact.rs               what a change affects: importers, layers, routes, tests
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
