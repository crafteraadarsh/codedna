# AGENTS.md

## Codebase Analysis

Before starting work, run:

```bash
codedna json . --compact
```

Use the output to confirm:

- `project_type` before making structural assumptions
- `frameworks` before adding dependencies or patterns
- `databases` before changing persistence code
- `architecture` before introducing new layers
- `file_breakdown` to locate central files quickly
- `dead_code` as a heuristic only; verify before deleting anything

## Working Rules

- Match the dominant language and existing framework choices.
- Prefer extending the current architecture over introducing new layers.
- Treat `dead_code` output as a hint, not proof.
