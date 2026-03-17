# GEMINI.md

## Project context

Run the following before starting any task:

```bash
codedna json . --compact
```

For remote repositories:

```bash
codedna json https://github.com/user/repo --compact
```

## Use the output fields

- **`project_type`** — overall project classification
- **`frameworks`** — frameworks in use (do not introduce duplicates)
- **`databases`** — databases connected (match existing patterns)
- **`infrastructure`** — DevOps tooling detected
- **`architecture`** — how layers communicate
- **`dead_code`** — unreferenced files (do not import from these)
- **`total_loc`** — scale of the codebase

## Rules

1. Do not introduce frameworks not already in `frameworks`
2. Follow the detected `architecture` pattern
3. Do not import from `dead_code` files
4. Match the dominant language for new files
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
