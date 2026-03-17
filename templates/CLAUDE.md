# CLAUDE.md

## Before Starting Work

Run:

```bash
codedna json . --compact
```

Use the output to understand:

- `project_type` before making structural assumptions
- `frameworks` before adding libraries or patterns
- `databases` before changing persistence code
- `architecture` before introducing new layers
- `file_breakdown` to find major files quickly
- `dead_code` as a heuristic only; verify before deleting anything

## Working Rules

- Match the existing framework and language choices.
- Prefer extending current patterns over adding new architecture.
- Treat `dead_code` as a hint, not proof.
