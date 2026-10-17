//! JavaScript and TypeScript module resolution through the real CLI:
//! package `exports` and `imports` with their conditions and patterns,
//! `tsconfig.json` `paths`, `baseUrl` and `extends`, workspace packages,
//! built output mapped back to source, declaration files next to their
//! JavaScript, and the fence around the repository.
//!
//! Every case comes as a pair: what must resolve and the look-alike that
//! must not. The expected targets were checked against reference traces on
//! the same files (with `node_modules` symlinks for the workspace packages,
//! which CodeDna does not need):
//!
//! - `tsc --traceResolution` (TypeScript 5.9.3, `moduleResolution:
//!   bundler`, `allowJs`), which applies the conditions `types`, `import`
//!   and `default`;
//! - Node.js 22 `require.resolve` and `import.meta.resolve`, plain and with
//!   `-C browser`.
//!
//! Where those consumers disagree (a `browser` build, a `node` build), the
//! report keeps each answer as a target labelled with its condition. The
//! one deliberate difference: built output that is not in the repository
//! (`main: ./dist/index.js`) resolves to its source, which neither tool
//! does without a build.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        // Old enough for the cache to keep it.
        let old = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
}

fn analyze(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), files);
    run(dir.path(), None)
}

fn run(dir: &Path, cache: Option<&Path>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codedna"));
    match cache {
        Some(cache) => {
            command.env("CODEDNA_CACHE_DIR", cache);
            command.env_remove("CODEDNA_NO_CACHE");
        }
        None => {
            command.arg("--no-cache");
        }
    }
    let output = command
        .args(["json", "--compact"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Windows reports `.\src\a.ts`; compare paths with forward slashes.
    let text = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\\\\", "/");
    serde_json::from_str(&text).unwrap()
}

fn facts(report: &Value) -> &Vec<Value> {
    report["dependencies"]["facts"].as_array().unwrap()
}

/// `(resolution, certainty, condition, targets)` of every fact for
/// `specifier`, in report order.
fn outcomes(report: &Value, specifier: &str) -> Vec<(String, String, String, Vec<String>)> {
    let found: Vec<_> = facts(report)
        .iter()
        .filter(|f| f["specifier"] == specifier)
        .map(|f| {
            let targets = f["targets"]
                .as_array()
                .map(|t| t.iter().map(|p| p.as_str().unwrap().to_string()).collect())
                .unwrap_or_default();
            (
                f["resolution"].as_str().unwrap().to_string(),
                f["certainty"].as_str().unwrap().to_string(),
                f["condition"].as_str().unwrap_or("").to_string(),
                targets,
            )
        })
        .collect();
    assert!(
        !found.is_empty(),
        "no fact for {specifier} in {:#}",
        report["dependencies"]
    );
    found
}

fn o(
    resolution: &str,
    certainty: &str,
    condition: &str,
    targets: &[&str],
) -> (String, String, String, Vec<String>) {
    (
        resolution.to_string(),
        certainty.to_string(),
        condition.to_string(),
        targets.iter().map(|t| t.to_string()).collect(),
    )
}

fn edges<'a>(report: &'a Value, source: &str) -> Vec<&'a str> {
    report["dependency_graph"][source]
        .as_array()
        .unwrap_or_else(|| panic!("{source} is not a node: {:#}", report["dependency_graph"]))
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect()
}

fn dead_code(report: &Value) -> Vec<&str> {
    let mut files: Vec<&str> = report["dead_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    files.sort();
    files
}

/// A workspace package with conditional and pattern `exports`.
const UI_PACKAGE: &str = r#"{
  "name": "@acme/ui",
  "type": "module",
  "exports": {
    ".": {"browser": "./src/browser.js", "node": "./src/node.js", "default": "./src/index.js"},
    "./esm": {"import": "./src/esm.js", "require": "./src/cjs.cjs"},
    "./feature/*": "./src/features/*.js",
    "./feature/secret": null,
    "./escape": "../outside.js"
  }
}"#;

fn ui_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "package.json",
            r#"{"name": "root", "private": true, "workspaces": ["packages/*", "apps/*"]}"#,
        ),
        ("packages/ui/package.json", UI_PACKAGE),
        ("packages/ui/src/browser.js", "export const browser = 1;\n"),
        ("packages/ui/src/node.js", "export const node = 1;\n"),
        ("packages/ui/src/index.js", "export const index = 1;\n"),
        ("packages/ui/src/esm.js", "export const esm = 1;\n"),
        ("packages/ui/src/cjs.cjs", "module.exports = 1;\n"),
        ("packages/ui/src/features/a.js", "export const a = 1;\n"),
        (
            "packages/ui/src/features/secret.js",
            "export const s = 1;\n",
        ),
        ("packages/outside.js", "export const o = 1;\n"),
        (
            "apps/web/package.json",
            r#"{"name": "web", "type": "module"}"#,
        ),
    ]
}

#[test]
fn unknown_conditions_keep_every_branch_labelled() {
    let mut files = ui_files();
    files.push((
        "apps/web/src/main.ts",
        "import { index } from \"@acme/ui\";\n",
    ));
    let report = analyze(&files);
    // tsc: index.js; node: node.js; node -C browser: browser.js.
    assert_eq!(
        outcomes(&report, "@acme/ui"),
        [
            o(
                "resolved",
                "direct",
                "browser",
                &["./packages/ui/src/browser.js"]
            ),
            o("resolved", "direct", "", &["./packages/ui/src/index.js"]),
            o("resolved", "direct", "node", &["./packages/ui/src/node.js"]),
        ]
    );
    let reachable = dead_code(&report);
    assert!(
        !reachable.contains(&"./packages/ui/src/browser.js"),
        "{reachable:?}"
    );
    assert!(
        !reachable.contains(&"./packages/ui/src/node.js"),
        "{reachable:?}"
    );
}

#[test]
fn import_and_require_conditions_follow_the_syntax() {
    let mut files = ui_files();
    files.push((
        "apps/web/src/a.ts",
        "import { esm } from \"@acme/ui/esm\";\n",
    ));
    files.push((
        "apps/web/src/b.cjs",
        "const cjs = require(\"@acme/ui/esm\");\n",
    ));
    let report = analyze(&files);
    // import: esm.js (tsc and node); require: cjs.cjs (node).
    assert_eq!(
        edges(&report, "./apps/web/src/a.ts"),
        ["./packages/ui/src/esm.js"]
    );
    assert_eq!(
        edges(&report, "./apps/web/src/b.cjs"),
        ["./packages/ui/src/cjs.cjs"]
    );
}

#[test]
fn export_patterns_resolve_and_blocked_or_missing_subpaths_do_not() {
    let mut files = ui_files();
    files.push((
        "apps/web/src/main.ts",
        "import { a } from \"@acme/ui/feature/a\";\n\
         import { s } from \"@acme/ui/feature/secret\";\n\
         import { n } from \"@acme/ui/nope\";\n",
    ));
    let report = analyze(&files);
    assert_eq!(
        outcomes(&report, "@acme/ui/feature/a"),
        [o(
            "resolved",
            "direct",
            "",
            &["./packages/ui/src/features/a.js"]
        )]
    );
    // `null` blocks the path; tsc and node both refuse it.
    assert_eq!(
        outcomes(&report, "@acme/ui/feature/secret"),
        [o("unresolved", "direct", "", &[])]
    );
    assert_eq!(
        outcomes(&report, "@acme/ui/nope"),
        [o("unresolved", "direct", "", &[])]
    );
    assert!(dead_code(&report).contains(&"./packages/ui/src/features/secret.js"));
}

#[test]
fn package_imports_resolve_and_unknown_hash_names_stay_unresolved() {
    let report = analyze(&[
        (
            "package.json",
            r##"{"name": "app", "type": "module", "imports": {"#db": "./src/db.ts", "#util/*": "./src/util/*.ts"}}"##,
        ),
        ("src/db.ts", "export const db = 1;\n"),
        ("src/util/u.ts", "export const u = 1;\n"),
        (
            "src/main.ts",
            "import { db } from \"#db\";\nimport { u } from \"#util/u\";\nimport { x } from \"#missing\";\n",
        ),
    ]);
    assert_eq!(
        edges(&report, "./src/main.ts"),
        ["./src/db.ts", "./src/util/u.ts"]
    );
    // A `#` name is always the package's own: unresolved, not external.
    assert_eq!(
        outcomes(&report, "#missing"),
        [o("unresolved", "direct", "", &[])]
    );
}

#[test]
fn paths_and_base_url_come_through_extends() {
    let report = analyze(&[
        (
            "tsconfig.base.json",
            r#"{"compilerOptions": {"baseUrl": ".", "paths": {"@lib/*": ["src/lib/*"]}}}"#,
        ),
        (
            "tsconfig.json",
            "{\n  // comments and trailing commas are fine\n  \"extends\": \"./tsconfig.base.json\",\n  \"compilerOptions\": {\"moduleResolution\": \"bundler\",},\n}\n",
        ),
        ("src/lib/math.ts", "export const m = 1;\n"),
        (
            "src/main.ts",
            "import { m } from \"@lib/math\";\n\
             import { m2 } from \"src/lib/math\";\n\
             import { g } from \"@lib/gone\";\n\
             import React from \"react\";\n",
        ),
    ]);
    assert_eq!(
        outcomes(&report, "@lib/math"),
        [o("resolved", "direct", "", &["./src/lib/math.ts"])]
    );
    assert_eq!(
        outcomes(&report, "src/lib/math"),
        [o("resolved", "direct", "", &["./src/lib/math.ts"])]
    );
    // A `paths` pattern matched, so the import is the project's own even
    // though no file exists; a package name is external.
    assert_eq!(
        outcomes(&report, "@lib/gone"),
        [o("unresolved", "direct", "", &[])]
    );
    assert_eq!(
        outcomes(&report, "react"),
        [o("external", "direct", "", &[])]
    );
    assert_eq!(report["coverage"]["imports"]["unresolved"], 1);
}

#[test]
fn duplicate_workspace_names_are_ambiguous_and_unique_ones_are_not() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name": "root", "private": true, "workspaces": ["packages/*"]}"#,
        ),
        (
            "packages/dup-a/package.json",
            r#"{"name": "dup", "main": "index.js"}"#,
        ),
        ("packages/dup-a/index.js", "module.exports = 1;\n"),
        (
            "packages/dup-b/package.json",
            r#"{"name": "dup", "main": "index.js"}"#,
        ),
        ("packages/dup-b/index.js", "module.exports = 2;\n"),
        (
            "packages/one/package.json",
            r#"{"name": "one", "main": "index.js"}"#,
        ),
        ("packages/one/index.js", "module.exports = 3;\n"),
        (
            "src/main.js",
            "const d = require(\"dup\");\nconst one = require(\"one\");\n",
        ),
    ]);
    // tsc and node pick whichever `node_modules/dup` links to.
    assert_eq!(
        outcomes(&report, "dup"),
        [o(
            "ambiguous",
            "direct",
            "",
            &["./packages/dup-a/index.js", "./packages/dup-b/index.js"]
        )]
    );
    assert_eq!(
        outcomes(&report, "one"),
        [o("resolved", "direct", "", &["./packages/one/index.js"])]
    );
    assert_eq!(
        edges(&report, "./src/main.js"),
        [
            "./packages/dup-a/index.js",
            "./packages/dup-b/index.js",
            "./packages/one/index.js"
        ]
    );
    assert_eq!(report["coverage"]["imports"]["ambiguous"], 1);
}

#[test]
fn same_named_packages_in_separate_workspaces_resolve_to_their_own() {
    let report = analyze(&[
        ("package.json", r#"{"name": "root", "private": true}"#),
        (
            "examples/a/pnpm-workspace.yaml",
            "packages:\n  - \"apps/*\"\n  - \"packages/*\"\n",
        ),
        (
            "examples/a/package.json",
            r#"{"name": "a", "private": true}"#,
        ),
        (
            "examples/a/packages/ui/package.json",
            r#"{"name": "@repo/ui", "main": "index.js"}"#,
        ),
        ("examples/a/packages/ui/index.js", "module.exports = 1;\n"),
        ("examples/a/apps/web/package.json", r#"{"name": "web-a"}"#),
        ("examples/a/apps/web/main.js", "require(\"@repo/ui\");\n"),
        (
            "examples/b/pnpm-workspace.yaml",
            "packages:\n  - \"apps/*\"\n  - \"packages/*\"\n",
        ),
        (
            "examples/b/package.json",
            r#"{"name": "b", "private": true}"#,
        ),
        (
            "examples/b/packages/ui/package.json",
            r#"{"name": "@repo/ui", "main": "index.js"}"#,
        ),
        ("examples/b/packages/ui/index.js", "module.exports = 2;\n"),
        (
            "examples/b/packages/ui2/package.json",
            r#"{"name": "@repo/ui", "main": "index.js"}"#,
        ),
        ("examples/b/packages/ui2/index.js", "module.exports = 3;\n"),
        ("examples/b/apps/web/package.json", r#"{"name": "web-b"}"#),
        ("examples/b/apps/web/main.js", "require(\"@repo/ui\");\n"),
    ]);
    assert_eq!(
        edges(&report, "./examples/a/apps/web/main.js"),
        ["./examples/a/packages/ui/index.js"]
    );
    // A real duplicate inside one workspace stays ambiguous, and the other
    // workspace's copy is no candidate.
    assert_eq!(
        edges(&report, "./examples/b/apps/web/main.js"),
        [
            "./examples/b/packages/ui/index.js",
            "./examples/b/packages/ui2/index.js"
        ]
    );
    assert_eq!(report["coverage"]["imports"]["ambiguous"], 1);
}

#[test]
fn built_output_maps_to_source_directly_only_with_tsconfig() {
    let report = analyze(&[
        (
            "packages/built/package.json",
            r#"{"name": "built", "main": "./dist/index.js"}"#,
        ),
        ("packages/built/src/index.ts", "export const b = 1;\n"),
        (
            "packages/cfg/package.json",
            r#"{"name": "cfg", "main": "./dist/index.js"}"#,
        ),
        (
            "packages/cfg/tsconfig.json",
            r#"{"compilerOptions": {"outDir": "dist", "rootDir": "src"}}"#,
        ),
        ("packages/cfg/src/index.ts", "export const c = 1;\n"),
        (
            "src/main.ts",
            "import { b } from \"built\";\nimport { c } from \"cfg\";\n",
        ),
    ]);
    assert_eq!(
        outcomes(&report, "built"),
        [o(
            "resolved",
            "inferred",
            "",
            &["./packages/built/src/index.ts"]
        )]
    );
    assert_eq!(
        outcomes(&report, "cfg"),
        [o(
            "resolved",
            "direct",
            "",
            &["./packages/cfg/src/index.ts"]
        )]
    );
    assert_eq!(report["coverage"]["imports"]["inferred"], 1);
}

#[test]
fn config_dir_in_a_shared_base_maps_built_output_from_the_package_root() {
    // The shape of trpc's examples: every package extends one base whose
    // `outDir` / `rootDir` use `${configDir}` (TypeScript 5.5), so
    // `dist/src/index.js` is built from `src/index.ts`.
    let report = analyze(&[
        ("pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n"),
        (
            "packages/tsconfig/package.json",
            r#"{"name": "@x/tsconfig", "files": ["base.json"]}"#,
        ),
        (
            "packages/tsconfig/base.json",
            r#"{"compilerOptions": {"outDir": "${configDir}/dist", "rootDir": "${configDir}"}}"#,
        ),
        (
            "packages/lib/package.json",
            r#"{"name": "@x/lib", "exports": {".": {"default": "./dist/src/index.js"}}}"#,
        ),
        (
            "packages/lib/tsconfig.json",
            r#"{"extends": ["@x/tsconfig/base.json"]}"#,
        ),
        ("packages/lib/src/index.ts", "export const l = 1;\n"),
        (
            "packages/bare/package.json",
            r#"{"name": "@x/bare", "exports": {".": {"default": "./dist/src/index.js"}}}"#,
        ),
        ("packages/bare/src/index.ts", "export const b = 1;\n"),
        (
            "packages/app/src/main.ts",
            "import { l } from \"@x/lib\";\nimport { b } from \"@x/bare\";\n",
        ),
    ]);
    assert_eq!(
        outcomes(&report, "@x/lib"),
        [o(
            "resolved",
            "direct",
            "",
            &["./packages/lib/src/index.ts"]
        )]
    );
    // No config says how `@x/bare` is built: the same file, guessed.
    assert_eq!(
        outcomes(&report, "@x/bare"),
        [o(
            "resolved",
            "inferred",
            "",
            &["./packages/bare/src/index.ts"]
        )]
    );
}

#[test]
fn javascript_next_to_its_typescript_source_is_what_runs() {
    let report = analyze(&[
        (
            "scripts/entry.js",
            "export { e } from \"./entry.ts\";\n",
        ),
        ("scripts/entry.ts", "export const e = 1;\n"),
        ("scripts/only.ts", "export const o = 1;\n"),
        (
            "tools/build.ts",
            "import { e } from \"../scripts/entry.js\";\nimport { o } from \"../scripts/only.js\";\nconsole.log(e, o);\n",
        ),
    ]);
    // tsc checks against entry.ts; Node.js and bundlers load entry.js.
    assert_eq!(
        outcomes(&report, "../scripts/entry.js"),
        [
            o("resolved", "direct", "", &["./scripts/entry.js"]),
            o("resolved", "direct", "types", &["./scripts/entry.ts"]),
        ]
    );
    assert_eq!(
        outcomes(&report, "../scripts/only.js"),
        [o("resolved", "direct", "", &["./scripts/only.ts"])]
    );
    // So the graph keeps entry.js reachable.
    assert_eq!(
        edges(&report, "./tools/build.ts"),
        [
            "./scripts/entry.js",
            "./scripts/entry.ts",
            "./scripts/only.ts"
        ]
    );
}

#[test]
fn declaration_next_to_javascript_is_the_types_target() {
    let report = analyze(&[
        ("src/legacy.js", "export const l = 1;\n"),
        ("src/legacy.d.ts", "export declare const l: number;\n"),
        ("src/a.ts", "import { l } from \"./legacy\";\n"),
        ("src/b.ts", "import type { l } from \"./legacy\";\n"),
    ]);
    // tsc: legacy.d.ts; a bundler or node runs legacy.js.
    let a: Vec<_> = facts(&report)
        .iter()
        .filter(|f| f["source"] == "./src/a.ts")
        .map(|f| {
            (
                f["condition"].as_str().unwrap_or(""),
                f["targets"][0].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(a, [("types", "./src/legacy.d.ts"), ("", "./src/legacy.js")]);
    assert_eq!(edges(&report, "./src/b.ts"), ["./src/legacy.d.ts"]);
}

#[test]
fn resolution_is_fenced_to_the_scanned_files() {
    let mut files = ui_files();
    files.push(("apps/web/dist/gen.js", "export const g = 1;\n"));
    files.push((
        "apps/web/src/main.ts",
        "import { g } from \"../dist/gen.js\";\n\
         import { e } from \"@acme/ui/escape\";\n\
         import { r } from \"react\";\n\
         import { l } from \"left-pad/sub\";\n",
    ));
    let report = analyze(&files);
    // `dist` is not scanned, so nothing in it is ever a target.
    assert_eq!(
        outcomes(&report, "../dist/gen.js"),
        [o("unresolved", "direct", "", &[])]
    );
    // An `exports` target must stay inside its package.
    assert_eq!(
        outcomes(&report, "@acme/ui/escape"),
        [o("unresolved", "direct", "", &[])]
    );
    assert_eq!(
        outcomes(&report, "react"),
        [o("external", "direct", "", &[])]
    );
    assert_eq!(
        outcomes(&report, "left-pad/sub"),
        [o("external", "direct", "", &[])]
    );
}

#[test]
fn executable_config_aliases_are_reported_as_unsupported() {
    let with = analyze(&[
        (
            "vite.config.ts",
            "export default {\n  resolve: { alias: { \"~\": \"/src\" } },\n};\n",
        ),
        ("src/main.ts", "export {};\n"),
    ]);
    let fact = facts(&with)
        .iter()
        .find(|f| f["evidence"]["rule"] == "js.config_alias_unsupported")
        .expect("unsupported alias fact");
    assert_eq!(fact["source"], "./vite.config.ts");
    assert_eq!(fact["resolution"], "unsupported");
    assert_eq!(fact["evidence"]["line"], 2);

    let without = analyze(&[
        (
            "vite.config.ts",
            "const aliases = [];\nexport default { plugins: aliases };\n",
        ),
        ("src/main.ts", "export {};\n"),
    ]);
    assert!(!facts(&without)
        .iter()
        .any(|f| f["evidence"]["rule"] == "js.config_alias_unsupported"));
}

#[test]
fn resolution_matches_with_the_cache_on_and_off() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut files = ui_files();
    files.push((
        "apps/web/src/main.ts",
        "import { index } from \"@acme/ui\";\nimport { a } from \"@acme/ui/feature/a\";\n",
    ));
    write(dir.path(), &files);
    let cold = run(dir.path(), None);
    let first = run(dir.path(), Some(cache.path()));
    let warm = run(dir.path(), Some(cache.path()));
    for report in [&first, &warm] {
        assert_eq!(report["dependencies"], cold["dependencies"]);
        assert_eq!(report["dependency_graph"], cold["dependency_graph"]);
        assert_eq!(report["coverage"], cold["coverage"]);
    }
    assert_eq!(
        cold["dependencies"]["rules_version"],
        json!(5),
        "rules version"
    );

    // A changed `exports` map is seen on the next cached run.
    let ui = UI_PACKAGE.replace(
        "\"default\": \"./src/index.js\"",
        "\"default\": \"./src/esm.js\"",
    );
    write(dir.path(), &[("packages/ui/package.json", &ui)]);
    let edited = run(dir.path(), Some(cache.path()));
    let targets = edges(&edited, "./apps/web/src/main.ts");
    assert!(targets.contains(&"./packages/ui/src/esm.js"), "{targets:?}");
    assert!(
        !targets.contains(&"./packages/ui/src/index.js"),
        "{targets:?}"
    );
}
