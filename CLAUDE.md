# CLAUDE.md

## Before starting any task

Run this first and read the output before writing any code:

```bash
codedna json . --compact
```

For remote repositories:

```bash
codedna json https://github.com/user/repo --compact
```

## Use the output to understand

- **`project_type`** — what kind of project this is
- **`frameworks`** — frameworks already in use (do not add duplicates)
- **`databases`** — databases connected (match existing patterns)
- **`infrastructure`** — DevOps tooling (Docker, GitHub Actions, etc.)
- **`architecture`** — how the layers communicate
- **`dead_code`** — files to avoid or safely delete
- **`total_loc`** — scale of the codebase

## Rules derived from analysis

1. Match detected frameworks — do not introduce new ones without asking
2. Follow the detected architecture pattern
3. Do not import from files listed in `dead_code`
4. Match the dominant language (highest LOC %) for new files
5. Match existing database and infrastructure patterns

## Quick context

```bash
codedna json . --compact | jq -c '{project_type,frameworks,databases,infrastructure,architecture,total_loc}'
```

## Skill reference

Full skill documentation: [skills/codedna-analyzer/SKILL.md](skills/codedna-analyzer/SKILL.md)

## Installation

```bash
cargo install --git https://github.com/crafteraadarsh/codedna
```
