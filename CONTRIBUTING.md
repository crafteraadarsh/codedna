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
│   ├── dependency_graph.rs   # Import/require/mod parser, emits dependency facts
│   ├── parsers/
│   │   ├── mod.rs            # Parse status and diagnostics shared by parsers
│   │   └── javascript.rs     # JS/TS/Vue/Svelte/Astro import scan on the Oxc syntax tree
│   ├── resolution/
│   │   ├── mod.rs            # Resolution outcomes, targets, order-preserving JSON
│   │   └── javascript.rs     # JS/TS resolution: package.json exports/imports, tsconfig, workspaces
│   ├── facts.rs              # Typed dependency facts (kind, resolution, evidence)
│   ├── graph.rs              # File graph built from resolved facts
│   ├── frameworks/           # Adapters: files Next.js, Expo Router, package.json and test runners load
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

### Changing JavaScript and TypeScript Import Rules

JS/TS imports are read from an [Oxc](https://oxc.rs) syntax tree in `src/parsers/javascript.rs`, never from lines of text. The README's "How it works" section lists every syntax that counts and its rule id; keep that table in step with the code.

1. Add a `RefKind` for the new syntax and recognise it in the `Collector` visitor, using syntax nodes and, where a name could be shadowed (`require`, `Worker`, loader functions), the semantic scope model.
2. Map it to a fact kind, certainty and rule id in `js_fact_kind` in `src/dependency_graph.rs`. Anything that is not proven to name one file is a `possibility` and must not add a graph edge.
3. Add a positive and a negative CLI test in `tests/cli_js_parser.rs`, and check the output with the cache on and off.
4. Bump `RULES_VERSION` in `src/facts.rs` and `FORMAT` in `src/cache.rs`, since cached results from the old rules are no longer valid.

Oxc crates are pinned to an exact version in `Cargo.toml`; upgrade them together and run the whole test suite.

### Adding or Changing a Framework Adapter

Adapters live in `src/frameworks/`, one file per framework, and name the files a framework loads without an import. The README's "How it works" section has the adapter matrix; keep it in step with the code.

1. Activate only on evidence in the package itself: its own `package.json` dependency, or its own config file. A dependency elsewhere in the repository (an example, a dev-only workspace) must not make files live.
2. Emit `configured_entry` facts through `Package::entry` with a versioned rule (`next.app_router@v1`), and register the adapter with `Package::adapter` so it appears in `dependencies.adapters`. Read only literal config values; never run a config file.
3. Write the tests first in `tests/cli_framework_adapters.rs`, as a pair: a file the adapter makes live, and the same file where the adapter is not active, which stays dead.
4. If a dead-code heuristic in `src/dead_code_detector.rs` covers the same files, make it defer to the adapter where the adapter is active.
5. Bump the rule's version when its meaning changes, bump `RULES_VERSION` and the cache `FORMAT`, check the output with the cache on and off, and measure dead code on a few real repositories before and after.

### Changing JavaScript and TypeScript Resolution

Specifiers are resolved to files in `src/resolution/javascript.rs`. The README's "How it works" section has the resolution matrix; keep it in step with the code.

1. Cite the rule you follow (a step of Node's ESM resolution algorithm, or a TypeScript option) in the code, and check the expected result with `tsc --traceResolution` or `node` on a small fixture. Do not claim equivalence where CodeDna deliberately differs (no `node_modules`, undecided conditions kept as alternatives).
2. Add a failing test first in `tests/cli_js_resolution.rs`, with a negative case next to the positive one.
3. A mapping CodeDna guesses (folder names, framework defaults) is `inferred`; only configuration or the file system makes it `direct`. Never read a file the scanner did not list.
4. Bump `RULES_VERSION` and the cache `FORMAT`, and check the output with the cache on and off.

Why not `oxc_resolver`: it resolves one condition set to one file, needs `node_modules` links for workspaces, has no notion of an inferred or ambiguous result, and would need a custom file system to stay inside the scanned files. CodeDna needs every condition branch, an honest certainty and a fence, so it has its own resolver.

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

The test suite currently has **370+ tests** across all modules. All tests must pass before a PR is merged.

If you change how dead code, audit findings or architecture are rated, check the measured confidence too. There are three labelled sets: `calibration/` (original), `calibration/fresh/` and `calibration/third/`, each with a `corpus.json` of pinned repositories and a `labels.json` of hand-checked results. Clone a set's repositories at their pinned commits into one folder, then run `codedna calibration --set original --corpus DIR` with your build. It scores those labels against what your build reports now.

The third set is the held-out one: tune and debug against the original and fresh sets only, and do not read `calibration/third/labels.json` while changing a detector. If a change is made after reading the third labels, say so in the PR and in the set's description in `src/calibration.rs`, because those numbers then stop being independent, and a new set is needed for an honest figure.

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