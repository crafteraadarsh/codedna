# Contributing to CodeDna 🧬

Thank you for your interest in contributing to CodeDna! This document covers everything you need to get started.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [How to Contribute](#how-to-contribute)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [Reporting Issues](#reporting-issues)

---

## Code of Conduct

Be respectful and constructive. We welcome contributors of all experience levels.

---

## Getting Started

### Prerequisites

- Rust stable toolchain (`rustup install stable`)
- `cargo` (bundled with Rust)
- `git`

### Clone and Build

```bash
git clone https://github.com/your-org/codedna
cd codedna
cargo build
cargo test
```

---

## Development Setup

```bash
# Build the project
cargo build

# Run all tests
cargo test

# Run a specific test module
cargo test -p codedna language_detector

# Run the binary locally
cargo run -- analyze .
cargo run -- json . | jq .

# Check for warnings
cargo clippy

# Format code
cargo fmt
```

---

## Project Structure

```
codedna/
├── src/
│   ├── main.rs               # Entry point — registers modules
│   ├── cli.rs                # clap CLI command definitions + dispatch
│   ├── analysis.rs           # AnalysisResult struct + full pipeline
│   ├── scanner.rs            # Recursive file walker (walkdir)
│   ├── language_detector.rs  # File extension → Language enum
│   ├── loc_counter.rs        # Non-empty line counter + binary detection
│   ├── framework_detector.rs # Manifest parser + per-file import scanner
│   ├── dependency_graph.rs   # Import/require/mod statement parser
│   ├── dead_code_detector.rs # BFS from entry points → unreachable files
│   ├── reporter.rs           # CLI report formatter + JSON serialiser
│   └── repo_map.rs           # Directory tree renderer
├── Cargo.toml
├── README.md
├── LICENSE
└── CONTRIBUTING.md
```

---

## How to Contribute

### Types of Contributions Welcome

| Type | Examples |
|---|---|
| Bug fixes | Incorrect detection, panics, wrong output |
| New language support | Add extensions to `language_detector.rs` |
| New framework detection | Add rules to `framework_detector.rs` per `Detectors.md` |
| New database detection | Add rules to `JS_DATABASE_RULES` / `PYTHON_DATABASE_RULES` |
| Performance improvements | Faster scanning, lower memory usage |
| Test coverage | More edge cases, real-world repo fixtures |
| Documentation | README improvements, inline doc comments |
| CLI improvements | New flags, better output formatting |

### Adding a New Framework

1. Add the package name and display label to the appropriate rule table in `framework_detector.rs`:
   - `JS_FRAMEWORK_RULES` — for `package.json`
   - `PYTHON_FRAMEWORK_RULES` — for `requirements.txt`
   - `RUST_CRATE_RULES` — for `Cargo.toml`
   - `GO_MODULE_RULES` — for `go.mod`

2. Add import patterns to `FRAMEWORK_FILE_PATTERNS` for per-file usage detection.

3. Add a unit test in the `#[cfg(test)]` block of `framework_detector.rs`.

4. Update `Detectors.md` with the new detection rule.

### Adding a New Language

1. Add a variant to the `Language` enum in `language_detector.rs`.
2. Add the extension mapping in the `match ext.as_str()` block.
3. Add a `fmt::Display` arm.
4. Add a unit test for the new extension.

---

## Coding Standards

### General

- Write idiomatic Rust — prefer `?` over `unwrap()`, use `Option`/`Result` for error handling.
- All public functions must have a doc comment (`///`).
- Keep modules focused — each `.rs` file should have one clear responsibility.
- No `unwrap()` or `expect()` in production paths (only in tests).

### Naming

- Functions: `snake_case`
- Types/Enums: `PascalCase`
- Constants: `SCREAMING_SNAKE_CASE`
- Module files: `snake_case.rs`

### Error Handling

- Unreadable files must be skipped silently (non-fatal).
- Malformed manifests (invalid JSON, bad TOML) must be skipped with a warning in verbose mode.
- The tool must never panic on user input.

### Performance

- New file-scanning loops should use `rayon::par_iter` where the operation is stateless.
- Avoid reading the same file more than once per pipeline stage.
- Avoid heap allocations in hot loops where possible.

### Tests

Every new feature must include at least one unit test. Tests must:
- Use `unique_temp_dir()` for isolated temp directories.
- Clean up with `fs::remove_dir_all(dir).ok()` at the end.
- Have descriptive names: `detects_react_from_package_json`, not `test1`.

---

## Testing

```bash
# Run all tests
cargo test

# Run tests for a specific module
cargo test -p codedna scanner
cargo test -p codedna framework_detector
cargo test -p codedna dead_code_detector

# Run a single test by name
cargo test -p codedna detects_react_from_package_json

# Run with output shown
cargo test -- --nocapture
```

The test suite currently has **140+ tests** across all modules. All tests must pass before a PR is merged.

---

## Submitting a Pull Request

1. **Fork** the repository and create a feature branch:
   ```bash
   git checkout -b feat/add-svelte-detection
   ```

2. **Make your changes** following the coding standards above.

3. **Add or update tests** — every change should have test coverage.

4. **Run the full test suite**:
   ```bash
   cargo test && cargo clippy && cargo fmt --check
   ```

5. **Commit with a descriptive message**:
   ```bash
   git commit -m "feat: add Svelte framework detection from package.json"
   ```

6. **Push and open a Pull Request** against `main`.

### PR Checklist

- [ ] `cargo test` passes with 0 failures
- [ ] `cargo clippy` has no errors
- [ ] `cargo fmt` applied
- [ ] New public functions have `///` doc comments
- [ ] `DETECTORS.md` updated if adding a new detection rule
- [ ] Tests added for new functionality

---

## Reporting Issues

When filing a bug report, please include:

1. **CodeDna version**: `codedna --version`
2. **Operating system and Rust version**: `rustc --version`
3. **Command run**: e.g. `codedna analyze ./my-project`
4. **Expected output** vs **actual output**
5. **Repository type** (if shareable): e.g. "Next.js monorepo with Prisma"

---

## Questions?

Open a GitHub Discussion or file an issue with the `question` label.

We appreciate every contribution — thank you for helping make CodeDna better! 🧬