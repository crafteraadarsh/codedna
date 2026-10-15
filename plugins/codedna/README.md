# CodeDna Claude Plugin

This plugin packages CodeDna for Claude Code. It adds:

- the `codedna` skill (`skills/codedna/`), which tells Claude when to use CodeDna
- the CodeDna MCP server (`.mcp.json`), which gives Claude read-only tools such as `get_context` and `get_stack`

The `codedna` binary must be installed and in your `PATH`. Install it with:

```bash
curl -fsSL https://raw.githubusercontent.com/crafteraadarsh/codedna/main/install.sh | sh
```

or, with Rust installed:

```bash
cargo install --git https://github.com/crafteraadarsh/codedna
```

Then, in Claude Code:

```text
/plugin marketplace add crafteraadarsh/codedna
/plugin install codedna@codedna-marketplace
```
