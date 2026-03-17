# AGENTS.md

## Codebase Analysis — run first on every task

Before starting any task in this repository, run the following command and read the output:

```bash
codedna json . --compact
```

For remote repositories, pass the URL directly:

```bash
codedna json https://github.com/user/repo --compact
```

## How to use the output

Extract and use these fields to inform your work:

- **`project_type`** — overall project classification (e.g., "Full-stack web application")
- **`frameworks`** — active frameworks (do not introduce duplicates)
- **`databases`** — databases in use (match existing patterns)
- **`infrastructure`** — DevOps tooling detected (Docker, CI, etc.)
- **`architecture`** — how layers communicate (e.g., "Frontend -> API -> Database")
- **`dead_code`** — unreferenced files (do not import from these)
- **`total_loc`** — scale of the codebase

## Rules

1. Do not introduce frameworks that are not already in `frameworks`
2. Follow the detected `architecture` pattern for new code
3. Do not import from files listed in `dead_code`
4. Match the dominant language (highest LOC) for new files
5. Match existing database patterns when adding data access code

## Quick context one-liner

```bash
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'
```

## Skill reference

Full skill documentation: [skills/codedna-analyzer/SKILL.md](skills/codedna-analyzer/SKILL.md)

## Installation

```bash
cargo install --git https://github.com/crafteraadarsh/codedna
```
