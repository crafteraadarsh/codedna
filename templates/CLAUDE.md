# CLAUDE.md

## Before Starting Work

Run:

```bash
codedna context .
```

If the CodeDna MCP tools are connected, call `get_context` instead.

Use the output to understand:

- the project type before making structural assumptions
- frameworks before adding libraries or patterns
- databases before changing persistence code
- architecture and entry points before introducing new layers
- the largest and most imported files to find the core quickly

## Working Rules

- Match the existing framework and language choices.
- Prefer extending current patterns over adding new architecture.
- Treat dead-code candidates as a hint, not proof.
