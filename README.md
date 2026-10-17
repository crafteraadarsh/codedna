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

codedna report . --format html -o report.html  # shareable report (print to PDF)
codedna json . > before.json                   # save a snapshot...
codedna diff before.json .                     # ...and see what changed later
```

## 🤖 Use with AI agents

CodeDna works with any agent that can run a shell command, and connects natively to agents that support MCP or skills.

**1. Context in one command.** `codedna context .` prints a short Markdown briefing: project type, stack, architecture, entry points, most imported files, largest files and a directory map. `--max-tokens N` changes the budget; `--format json` gives the same data as JSON. `--task "add rate limiting to login"` opens the briefing with the files that matter for that task: files whose names or code mention its words, the files they import or are imported by, and related tests, each with why it was picked (MCP: the `task` argument of `get_context`). Matching is by wording, not meaning.

**2. MCP server.** `codedna mcp` serves read-only tools over stdio:

<p>
<code>get_context</code> · <code>analyze_repo</code> · <code>get_stack</code> · <code>list_files</code> · <code>find_framework_usage</code> · <code>find_dead_code</code> · <code>find_circular_imports</code> · <code>repo_map</code> · <code>find_hotspots</code> · <code>audit_repo</code> · <code>get_runbook</code> · <code>get_topology</code> · <code>list_packages</code> · <code>get_impact</code>
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
| `codedna report [path\|url] -o FILE` | Shareable Markdown report (`--format html` for a self-contained page) |
| `codedna diff <before> [after]` | Stack, size and file changes between two snapshots, paths or URLs (`--json`, `--markdown`) |
| `codedna json [path\|url]` | Full analysis as JSON (`--compact`, `--time`) |
| `codedna schema` | JSON Schema of the `json` report |
| `codedna calibration` | How often each confidence level was right on hand-checked repositories (`--json`, `--set third\|fresh\|original`, `--corpus DIR` to re-measure this build) |
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
  "analyzer": { "name": "codedna", "version": "0.2.11" },
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
| `analyzer` | object | `name` and `version` of the CodeDna build that wrote it |
| `project_type` | string | High-level project classification |
| `total_loc` | number | Lines of code, excluding comments, blank lines and lockfiles |
| `comment_lines` | number | Comment-only lines across detected source files |
| `blank_lines` | number | Blank lines across detected source files |
| `languages` | object | Language → lines of code |
| `frameworks` | string[] | Detected frameworks |
| `databases` | string[] | Detected databases and ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Architecture summary from detected frameworks and databases |
| `architecture_details` | object | Layers found in the code (`layers`, each with `confirmed_by_code`, `placed_by_imports` and up to 3 `examples`), import counts between them (`flows`, each with up to 3 example imports), `confidence` (`high`/`medium`/`low`) and `evidence` |
| `dead_code` | string[] | Files nothing reaches from an entry point |
| `dead_code_details` | object[] | Per file `{ file, confidence, reason, evidence }`: how likely it is to be really unused, why, and the lines behind the reason (`{ file, line, text }`, up to 3) |
| `circular_imports` | string[][] | Groups of files that import each other, directly or through other files |
| `workspaces` | object[] | Monorepo workspaces declared at the root: `{ tool, manifest, packages: [{ name, path, files, loc, languages, frameworks, databases, dead_files, depends_on, used_by }] }` |
| `dependency_graph` | object | File → imported files. A projection of `dependencies`: the targets of its resolved facts |
| `dependencies` | object | Typed dependency facts: `rules_version` and `facts`, one per dependency found, sorted by source and position. Each has `source`, `specifier` (as written), `targets` (files it resolved to), `kind` (`runtime_import`, `type_only_import`, `re_export`, `worker_entry`, `process_entry`, `resource_reference`, `configured_entry`, `possible_dynamic_load`), `resolution` (`resolved`, `external`, `unresolved`, `ambiguous`, `unsupported`), `certainty` (`direct`, `declaration`, `inferred`, `possibility`), an optional `condition` (`dynamic` for `import()`) and `evidence` (`file`, `line`, byte `span` where known, `rule` id such as `js.import` or `python.from_import`). Unresolved and external imports are kept |
| `coverage` | object | How much the dependency analysis covered: `source_files`, `analyzed_files` (imports followed), `unreadable_files`, `not_analyzed` (languages whose imports are not followed, `{ language, files }`), `imports` (`{ internal, resolved, unresolved, external }`), `unresolved` (up to 20 `{ file, line, import }`) and a one-line `summary`. Read it before trusting an empty `dead_code` or `circular_imports` |
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

Paths matched by the repository's `.gitignore` files (root and nested) and `.ignore` files are skipped. `node_modules/`, `.git/`, `target/`, `dist/`, `build/` and `coverage/` are always skipped, as are files over 10 MiB and symbolic links.

</details>

<details>
<summary><b>Accuracy notes</b></summary>

<br>

- **Architecture** comes from two sources. The summary line uses detected frameworks and databases; manifests in `examples/`, `demo/`, `docs/` and test folders count only when the repository has no others, so a library's sample apps don't make it read as a web service. `architecture_details` then checks every source file, tests left out. A file is placed in a layer by its code: route handlers (Express, Nest, FastAPI, Django, Go, Spring, ASP.NET, Rails, Laravel, Rust), middleware signatures, ORM models and tables, and service annotations. When the code shows nothing, its folder or file name places it (`routes/`, `user.controller.ts`, `services/`, `models.py`, …). Code wins over a name that disagrees (tables kept in `services/` are Data), and the example records the name it overruled (`name_said`: `services/ (Services)`); the one exception is a handler in a routes folder that has the `(req, res, next)` middleware signature, which stays in Routes. A file with no other signal that routes import and that imports data is placed in Services. Imports between layers are then counted. Files in example, demo and docs folders are left out too, unless nothing else has layers. Confidence is high when layers import each other top-down (at least three imports down for each one up), or when every layer's name is backed by its code; medium when layers exist without that support; and low when none are found. Each layer reports `confirmed_by_code` and `placed_by_imports`, and shows up to 3 example files. An example says how it was placed (`placed_by`: `settings`, `name`, `code` or `imports`), what matched (the folder or file name, the `.codedna.toml` entry, or the files it imports and is imported by) and, when the code agrees, the line and sign (`code`: `{ line, sign }`, such as a route `GET /users` or `@Entity(`; line 0 means the whole file, as for a `.vue` component). Code-backed examples come first.
- **Unusual layouts** can name their layers in a `.codedna.toml` at the repository root. Keys are `ui`, `routes`, `middleware`, `services` and `data`; values are path prefixes. The longest prefix wins, and settings win over names and code:

  ```toml
  [layers]
  routes = ["src/http/"]
  data = ["src/store/", "src/tables.rs"]
  ```
- **Lines of code** count only lines that contain code. Comment-only and blank lines are reported separately (`comment_lines`, `blank_lines`), and comment markers inside strings are not treated as comments. Lockfiles (`package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock` and similar) are left out so they don't swamp the totals.
- **Dead code is a heuristic, with a confidence for each file.** A file is listed when nothing reaches it through imports from an entry point. Each one is then rated with a reason. *High*: its name appears nowhere else in the repository. *Medium*: nothing imports it, but its name is mentioned elsewhere or the repository loads modules dynamically. *Low*: it is probably loaded another way, for example from a convention folder (`migrations/`, `alembic/`, `management/`, `scripts/`, …), as a test, as a Django app module (`models.py`, `admin.py`, `signals.py`, …), by a dynamic `import()`/`require()`/`importlib` call that names it, or by a path built from a template string (`` `index-drizzle/with-${db}.ts` ``) that matches its folder and name. Python dotted strings in settings (`"app.views"`) count as imports. When the reason rests on lines elsewhere, such as files that mention the name or the dynamic loader, `evidence` gives up to 3 of them with file, line and text. Check before deleting anything, even at high confidence.
- **Files loaded by name count only when code loads them.** Besides imports, a JS/TS file is reached when it is the first argument of `new URL(..., import.meta.url)` or `new Worker(...)`/`new SharedWorker(...)` (unless the file defines or imports its own `Worker`), or is named among the arguments of `spawn`, `fork`, `exec`, `execFile` and their `Sync` forms imported from `child_process`, of `execa` or `cross-spawn`, or of `Bun.spawn`. The file must be named with its extension and is looked up next to the calling file. A path in a log message, comment or any other string, or passed to a local function that shares a loader's name, is not a dependency. Paths built at runtime and loaders reached through other wrappers are not followed.
- **Code signs count only in code.** Words such as `@RestController`, `pgTable(` or `@app.get(` inside a string, a comment or a test fixture are not taken as evidence for a layer or a route.
- **Workspace packages resolve through their `package.json`.** An import of a local package (`@acme/shared`, `@acme/ui/button`) follows its `exports` (plain, conditional and `*` patterns), then `source`, `module`, `main` or `types`, then `src/index`. Targets in a build folder (`dist/`, `build/`, `lib/`, …) map back to the matching file under `src/`.
- **Coverage** says how much of the code the dependency graph covers, so an empty dead-code or cycle list can be told apart from code that was never looked at. It counts the source files whose imports were followed, imports of the repository's own code that matched no file (relative paths, path aliases, workspace packages, the project's own Python packages and `mod` declarations), with file and line for the first 20, and languages whose imports are not followed. The terminal report, `context`, reports and MCP tools show it; `deadcode` and `cycles` say "not checked" when no file could be analyzed.
- **Audit** matches known risky patterns. It cannot prove code is safe: a clean result only means none of its checks matched. Each finding has a severity (how bad it would be) and a confidence (how likely it is real, not a false alarm). Findings in tests, docs, examples, fixtures, vendored libraries (`vendor/`, `.yarn/`, `*.min.js`, `*-bundle.js`) and translations (`locales/`, `i18n/`) are lowered one step and marked with their `context`; keys in any of them drop to low confidence, and long lines, encoded data and direction marks in tests, fixtures, vendored libraries and translations are not reported.
- **Monorepos**: workspaces declared at the repository root are listed with their packages (`workspaces` in JSON): npm, Yarn and Bun workspaces, pnpm (`pnpm-workspace.yaml`), Lerna, Cargo, Go (`go.work`) and uv. Each package gets its own size, languages, stack, dead files and links to the other packages (`codedna packages`).
- **Routes** in `runbook` come from common framework patterns and may be incomplete.
- **Topology** reads deployment files as written; nothing is rendered or deployed. Helm values are not applied, so templated names stay as `{{ … }}`, and Kustomize overlays are listed as files, not merged. A Service links to the workloads its selector matches, preferring ones in the same file, then the same folder. Code is found from Skaffold and Compose build contexts, an image named like a folder with a Dockerfile, or a function's handler. Files in test, example and docs folders are skipped and counted.
- **Change impact** follows the same imports as the dependency graph, backwards from the changed files, so a file reached another way (configuration, reflection, a language whose imports are not followed) is not listed. Untracked files count as added. Tests are files that look like tests (`*.test.ts`, `test_*.py`, `tests/`, …) and import changed code, directly or through other files.

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

- **Dead code is a lead at every level, including high.** Most wrong ones are files a tool or framework loads without an import: by file-name convention (Expo and Next.js special files, Alembic and TypeORM migrations, test setup and mocks, type tests), by folder scan (`import.meta.glob`, plugin and migration folders), by a script in `package.json` or CI, or by a module name in settings. A few were imported by a relative path CodeDna missed.
- **Audit findings are mostly noise outside high.** Most wrong ones were vendored or minified code (such as Yarn's own `.yarn/releases` bundle), test fixtures and translation strings.
- **Architecture** is the most reliable: when CodeDna names layers at high confidence it was right 28 times out of 32.
- One bug found while labelling was fixed in v0.2.9: a Django app without an `apps.py` was reported as having a dead `apps.py`. The numbers above are for v0.2.8, before that fix.

**Original set.** 31 other repositories, labelled with v0.2.6 (301 results): dead code high 59% (29/49), medium 54% (26/48), low 12% (12/103); audit 2/4, 9/22, 2/33; architecture 16/17, 2/10, 4/4. v0.2.7 fixed detectors after reading these labels, which raised high-confidence dead code on that set to 97% (29/30). That figure is not independent, and the fresh set shows it does not carry over to new repositories.

`codedna calibration` prints all three sets with their intervals (`--set third` for one). The repositories and labels are in [`calibration/`](calibration/); clone a set and run `codedna calibration --set third --corpus DIR` to score your own build on the same labels.

## ⚡ Performance

`codedna analyze` with v0.1.3 on a 2-core Linux machine, best of 5 runs:

| Repository | Files | Lines of code | Time |
|---|---:|---:|---:|
| [tokio](https://github.com/tokio-rs/tokio) | 886 | 114k | **60 ms** |
| [fastapi](https://github.com/fastapi/fastapi) | 2,993 | 255k | **186 ms** |
| [django](https://github.com/django/django) | 5,699 | 468k | **631 ms** |

Lines of code exclude comments and blank lines.

**Repeat runs are incremental.** CodeDna remembers what each file's content showed (line counts, imports, layer, dead-code mentions) and reads again only the files whose size or modification time changed. On a 2-core Linux machine with v0.2.5:

| Repository | Files | First run | Repeat run |
|---|---:|---:|---:|
| [vscode](https://github.com/microsoft/vscode) | 20,744 | 11.6 s | **1.4 s** |
| [kubernetes](https://github.com/kubernetes/kubernetes) | 31,461 | 4.9 s | **0.9 s** |

The output is identical either way. Adding, removing or renaming a file, or editing a `.json` / `.toml` file, re-resolves every file's imports, since any of them could point at it. The cache is kept in your user cache folder (`~/.cache/codedna`, `~/Library/Caches/codedna` on macOS, `%LOCALAPPDATA%\codedna\cache` on Windows; `CODEDNA_CACHE_DIR` moves it), never in the repository, and remote clones are never cached. `--no-cache` or `CODEDNA_NO_CACHE=1` reads every file afresh.

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
├── facts.rs                typed dependency facts
├── graph.rs                dependency graph built from facts
├── dead_code_detector.rs   reachability from entry points
├── context.rs              token-budgeted briefing for agents
├── mcp.rs                  MCP server (JSON-RPC over stdio)
├── hotspots.rs             churn × size ranking, co-change coupling
├── audit.rs                supply-chain red-flag scan
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
