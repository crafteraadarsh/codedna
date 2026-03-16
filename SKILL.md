# CodeDna Skill 🧬

```yaml
name: codedna
version: 2.0.0
description: >
  Analyze a repository and reveal its codebase DNA —
  tech stack, architecture, LOC distribution, framework usage,
  infrastructure, and dead code. Outputs structured JSON for agent consumption.
repository: https://github.com/crafteraadarsh/codedna
```

---

## Purpose

CodeDna is a **codebase intelligence skill** for AI coding agents and developers.
Before reading a single source file, an agent can run CodeDna to instantly
understand what it's working with.

It detects and reports:
- Programming languages and LOC distribution (with visual bar chart)
- Frameworks across JS/TS, Python, Rust, and Go ecosystems
- Databases and data-layer technologies
- Infrastructure and DevOps tooling (Docker, GitHub Actions, Kubernetes)
- Architecture patterns (Frontend → API → Database)
- Per-file framework import usage
- Unused / dead code via dependency graph traversal
- Directory tree structure

---

## Supported Agents

| Agent | Status | Config file |
|---|---|---|
| Claude Code | ✅ Supported | `CLAUDE.md` |
| OpenCode | ✅ Supported | `AGENTS.md` |
| Codex CLI | ✅ Supported | `AGENTS.md` |
| Cursor | ✅ Supported | `.cursorrules` |
| Gemini CLI | ✅ Supported | `GEMINI.md` |

---

## Installation

```bash
# Install from GitHub (works immediately)
cargo install --git https://github.com/crafteraadarsh/codedna

# Or from source
git clone https://github.com/crafteraadarsh/codedna
cd codedna
cargo install --path .
```

Requires **Rust stable** — install at [rustup.rs](https://rustup.rs).

Verify installation:

```bash
codedna --version
# codedna 0.1.0
```

---

## When To Use This Skill

Trigger CodeDna when the user asks any of the following:

| User intent | Command |
|---|---|
| *"What tech stack does this repo use?"* | `codedna stack .` |
| *"Explain this repository"* | `codedna analyze .` |
| *"Show the architecture of this project"* | `codedna analyze .` |
| *"How many lines of code?"* | `codedna files .` |
| *"Find unused or dead code"* | `codedna deadcode .` |
| *"Where is React used?"* | `codedna framework react .` |
| *"Show me the directory structure"* | `codedna map .` |
| *"Give me machine-readable output"* | `codedna json .` |

---

## Commands

### Full Intelligence Analysis
```bash
codedna analyze .
codedna analyze . --time        # also print elapsed time
```
Produces the complete formatted intelligence report: project type, stack, databases,
infrastructure, architecture, LOC bar chart, top 5 files, dead code, and summary.

### Tech Stack Only
```bash
codedna stack .
```
Returns detected languages (with LOC %), frameworks, and databases.

### File-Level LOC Breakdown
```bash
codedna files .
```
Returns per-file LOC breakdown sorted by size descending.

### Framework Usage (per-file)
```bash
codedna framework <name> [path]
# Examples:
codedna framework react .
codedna framework fastapi ./backend
codedna framework axum .
```
Returns all files where the specified framework is imported/used.

### Dead Code Detection
```bash
codedna deadcode .
```
Returns list of files unreachable from any entry point via dependency graph traversal.

### Repository Map
```bash
codedna map .
codedna map . --depth 3
```
Renders a tree-style directory structure respecting ignore rules.

### Machine-Readable JSON (Agent Use)
```bash
codedna json .
codedna json . --compact         # single-line JSON
codedna json . --time            # also print elapsed time to stderr
```
Returns the full analysis as structured JSON. Intended for agent consumption.

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
  "databases": ["PostgreSQL"],
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
    { "file": "src/server.ts", "loc": 340, "language": "TypeScript" },
    { "file": "src/App.tsx",   "loc": 210, "language": "TypeScript" }
  ]
}
```

### Field Reference

| Field | Type | Description |
|---|---|---|
| `project_type` | string | High-level project classification |
| `total_loc` | number | Total non-empty lines of code |
| `languages` | object | Language → LOC count map |
| `frameworks` | string[] | Detected application frameworks |
| `databases` | string[] | Detected databases / ORMs |
| `infrastructure` | string[] | Detected DevOps tooling |
| `architecture` | string | Inferred architecture pattern |
| `dead_code` | string[] | Unreachable file paths |
| `dependency_graph` | object | File → imported files map |
| `file_breakdown` | object[] | Per-file `{ file, loc, language }` entries |

---

## Agent Integration Guide

### Claude Code

Create a `CLAUDE.md` file in your project root:

```markdown
# CLAUDE.md

## Before starting any task

Always run this first and read the output before writing any code:

```bash
codedna json . --compact
```

Use the output to understand:
- `project_type` — what kind of project this is
- `frameworks` — what frameworks are in use (don't add duplicates)
- `databases` — what databases are connected
- `architecture` — how the layers are arranged
- `dead_code` — files to avoid or that can be safely deleted
- `total_loc` — scale of the codebase
```

Or add to your global Claude Code settings (`~/.claude/CLAUDE.md`):

```markdown
## Codebase analysis (all projects)
When starting work on any new repository, run:
  codedna json . --compact
Parse the JSON output before suggesting any code changes.
```

---

### OpenCode

Create an `AGENTS.md` file in your project root:

```markdown
# AGENTS.md

## Codebase Analysis — run first on every task

Before starting any task, run the following command and read the output:

```bash
codedna json . --compact
```

Extract and use these fields to inform your work:
- `project_type` — overall project classification
- `frameworks` — active frameworks (do not introduce duplicates)
- `databases` — databases in use (match existing patterns)
- `architecture` — how layers communicate
- `dead_code` — unreferenced files (do not import from these)
```

OpenCode reads `AGENTS.md` automatically at the start of every session.

---

### Codex CLI

Add to your `AGENTS.md` (same format as OpenCode above), or pass context inline:

```bash
# Inject CodeDna context directly into a Codex CLI prompt
codedna json . --compact | codex "Here is the codebase context: $(cat -). Now add a user authentication endpoint."
```

Or pipe and store for reuse:

```bash
# Save context to a temp file and reference in prompts
codedna json . --compact > /tmp/repo-context.json
codex --context /tmp/repo-context.json "Refactor the authentication module"
```

---

### Cursor

Add to your `.cursorrules` file in the project root:

```
# Cursor Rules

## Project Analysis
Before suggesting any code, run `codedna json . --compact` in the terminal
and use the output to understand the project stack, architecture, and dead code.

## Rules derived from CodeDna output
- Match the detected frameworks — do not introduce new ones without asking
- Follow the detected architecture pattern
- Do not import from files listed in dead_code
- Match the dominant language (highest LOC) for new files
```

---

### Gemini CLI

Create a `GEMINI.md` file in your project root:

```markdown
# GEMINI.md

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

Regardless of the agent you're using, you can always inject context manually:

```bash
# Copy compact JSON context to clipboard (macOS)
codedna json . --compact | pbcopy

# Copy compact JSON context to clipboard (Linux)
codedna json . --compact | xclip -selection clipboard

# Print a human-readable summary
codedna analyze .
```

Then paste it into your agent's chat window as the first message.

---

## Useful jq Recipes for Agents

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

## Architecture Inference

| Signal combination | Output |
|---|---|
| Frontend + Backend + Database | `Frontend → API → Database` |
| Frontend + Backend | `Frontend → API` |
| Backend + Database | `API → Database` |
| Frontend + Database (no backend) | `Frontend → Database` |
| Frontend only | `Frontend only` |
| Backend only | `API only` |
| None of the above | `Monolithic / undetermined` |

---

## Project Type Inference

| Detected signals | Project type |
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

## Ignored Directories

The scanner always skips:

```
node_modules/   .git/   target/   dist/   build/   coverage/
```

---

## Performance

| Metric | Value |
|---|---|
| Analysis time (debug build, ~26 files) | ~2.6 ms |
| Parallelism | rayon thread pool (all CPU cores) |
| Target for 100k+ file repos | < 5 seconds |
| Memory model | Streaming per-file reads, no full-repo buffer |

---

## Links

- **Repository:** https://github.com/crafteraadarsh/codedna
- **Install:** `cargo install --git https://github.com/crafteraadarsh/codedna`
- **Issues:** https://github.com/crafteraadarsh/codedna/issues
- **Contributing:** See [CONTRIBUTING.md](CONTRIBUTING.md)