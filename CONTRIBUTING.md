# Contributing to CodeDna 🧬

Thank you for your interest in contributing to CodeDna! This document covers everything you need to get started.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Your First PR in 15 Minutes](#your-first-pr-in-15-minutes)
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

## Your First PR in 15 Minutes

New to Rust or open source? Adding a detector is the easiest way in. Most are a one-line rule plus one test.

1. Pick an issue labelled [`good first issue`](https://github.com/crafteraadarsh/codedna/labels/good%20first%20issue) and comment that you're taking it.
2. Open `rules.toml` in the repo root, find the section the issue names (for example `[npm.frameworks]`) and add one line:

   ```toml
   "@angular/core" = "Angular"
   ```

   No Rust changes are needed. The rules are built into the binary.

3. Copy the nearest existing test in `src/framework_detector.rs` (for example `detects_react_from_package_json`), rename it and change the package and label:

   ```rust
   #[test]
   fn detects_angular_from_package_json() {
       let dir = unique_temp_dir("codedna_fw_angular");
       let file = dir.join("package.json");
       fs::write(&file, r#"{ "dependencies": { "@angular/core": "^18.0.0" } }"#).unwrap();
       let result = detect_frameworks(&[file]);
       assert!(result.frameworks.contains(&"Angular".to_string()));
       fs::remove_dir_all(dir).ok();
   }
   ```

4. Run `cargo test && cargo clippy -- -D warnings && cargo fmt --check`, commit, and open a PR that says `Closes #<issue>`.

Stuck? Comment on the issue. Questions are welcome.

---

## Getting Started

### Prerequisites

- Rust stable toolchain (`rustup install stable`)
- `cargo` (bundled with Rust)
- `git`

### Clone and Build

Fork the repository on GitHub, then:

```bash
git clone https://github.com/<your-username>/codedna
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
├── rules.toml                # All detection rules (languages, frameworks, databases, infra)
├── src/
│   ├── main.rs               # Entry point — registers modules
│   ├── rules.rs              # Loads and validates rules.toml
│   ├── cli.rs                # clap CLI command definitions + dispatch
│   ├── analysis.rs           # AnalysisResult struct + full pipeline
│   ├── scanner.rs            # Recursive file walker (walkdir)
│   ├── language_detector.rs  # File extension → Language (from rules.toml)
│   ├── loc_counter.rs        # Non-empty line counter + binary detection
│   ├── framework_detector.rs # Manifest parser + per-file import scanner
│   ├── dependency_graph.rs   # Import/require/mod statement parser
│   ├── dead_code_detector.rs # BFS from entry points → unreachable files
│   ├── reporter.rs           # CLI report formatter + JSON serialiser
│   ├── repo_map.rs           # Directory tree renderer
│   └── safe_text.rs          # Strips terminal escapes from printed names
├── Cargo.toml
├── README.md
├── LICENSE
├── CONTRIBUTING.md
└── SECURITY.md
```

---

## How to Contribute

### Types of Contributions Welcome

| Type | Examples |
|---|---|
| Bug fixes | Incorrect detection, panics, wrong output |
| New language support | Add a `[[language]]` entry to `rules.toml` |
| New framework detection | Add a line under `[<ecosystem>.frameworks]` in `rules.toml` |
| New database detection | Add a line under `[<ecosystem>.databases]` in `rules.toml` |
| New infrastructure detection | Add an `[[infra]]` entry to `rules.toml` |
| Performance improvements | Faster scanning, lower memory usage |
| Test coverage | More edge cases, real-world repo fixtures |
| Documentation | README improvements, inline doc comments |
| CLI improvements | New flags, better output formatting |

### Adding a New Framework or Database

1. Add `package = "Label"` to the right section of `rules.toml`:
   - `[npm.frameworks]` / `[npm.databases]` for `package.json`
   - `[pypi.frameworks]` / `[pypi.databases]` for `requirements.txt` and `pyproject.toml` (lowercase names)
   - `[cargo.frameworks]` / `[cargo.databases]` for `Cargo.toml`
   - `[go.frameworks]` / `[go.databases]` for `go.mod` (the key is a module path prefix)

   Quote keys that contain `@`, `/` or `.`, like `"@nestjs/core" = "NestJS"`.

2. Optionally add a `[[file_pattern]]` entry so `codedna framework <name>` can find the files that import it.

3. Add a unit test in the `#[cfg(test)]` block of `src/framework_detector.rs`.

4. If it's a new manifest type, mention it in the README's detection list.

### Adding a New Language

1. Add an entry to `rules.toml`:

   ```toml
   [[language]]
   name = "Kotlin"
   extensions = ["kt", "kts"]
   ```

   Extensions are lowercase, without the dot, and may belong to only one language.

2. Add a unit test in `src/language_detector.rs`:

   ```rust
   #[test]
   fn detects_kotlin() {
       assert_eq!(detect_language(&p("Main.kt")), Some(Language("Kotlin")));
   }
   ```

### Adding Infrastructure Detection

Add an `[[infra]]` entry to `rules.toml` with any of `files` (exact file names), `dirs` (directories the file sits under) and `extensions`, then a test in `src/framework_detector.rs`:

```toml
[[infra]]
name = "Terraform"
extensions = ["tf"]
```

`cargo test` also checks `rules.toml` itself, so typos like a duplicated extension or an uppercase PyPI name fail with a clear message.

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
- [ ] `cargo clippy -- -D warnings` has no warnings
- [ ] `cargo fmt` applied
- [ ] New public functions have `///` doc comments
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