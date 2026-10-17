//! Folders named `build`, `dist`, `target` or `coverage` through the real
//! CLI: skipped where they hold build output, scanned where they are part
//! of the source tree (withastro/astro keeps its build step in
//! `packages/astro/src/core/build`).
//!
//! Every case comes as a pair: the source folder that must be scanned and
//! the output folder that must not.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }
}

fn analyze(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), files);
    let output = Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(["json", "--compact", "--no-cache"])
        .arg(dir.path())
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

/// Repository-relative paths of every file the report counted.
fn scanned(report: &Value) -> Vec<String> {
    report["file_breakdown"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            f["file"]
                .as_str()
                .unwrap()
                .trim_start_matches("./")
                .to_string()
        })
        .collect()
}

fn dead(report: &Value) -> Vec<String> {
    report["dead_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().trim_start_matches("./").to_string())
        .collect()
}

fn resolution(report: &Value, specifier: &str) -> String {
    report["dependencies"]["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["specifier"] == specifier)
        .map(|f| f["resolution"].as_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("no fact for {specifier}"))
}

const WORKSPACE: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    ),
    (
        "packages/a/package.json",
        r#"{"name":"a","main":"./src/index.ts"}"#,
    ),
    (
        "packages/a/src/index.ts",
        "import { b } from \"./core/build/index\";\nconsole.log(b);\n",
    ),
    (
        "packages/a/src/core/build/index.ts",
        "export const b = 1;\n",
    ),
    (
        "packages/a/src/core/build/orphan.ts",
        "export const unused = 2;\n",
    ),
];

#[test]
fn build_folder_inside_src_is_scanned_and_resolved() {
    let report = analyze(WORKSPACE);
    let files = scanned(&report);
    assert!(
        files.contains(&"packages/a/src/core/build/index.ts".to_string()),
        "{files:?}"
    );
    assert_eq!(resolution(&report, "./core/build/index"), "resolved");
    let dead = dead(&report);
    assert!(
        !dead.contains(&"packages/a/src/core/build/index.ts".to_string()),
        "{dead:?}"
    );
    // A file nobody imports in such a folder is now seen, and found unused.
    assert!(
        dead.contains(&"packages/a/src/core/build/orphan.ts".to_string()),
        "{dead:?}"
    );
}

#[test]
fn output_folders_at_package_and_repository_roots_stay_skipped() {
    let mut files = WORKSPACE.to_vec();
    files.extend_from_slice(&[
        ("dist/bundle.js", "var x = 1;\n"),
        ("build/out.js", "var y = 1;\n"),
        ("coverage/lcov-report/prettify.js", "var z = 1;\n"),
        ("packages/a/dist/index.js", "var w = 1;\n"),
        ("packages/a/build/index.js", "var v = 1;\n"),
        ("crates/c/Cargo.toml", "[package]\nname = \"c\"\n"),
        ("crates/c/src/lib.rs", "pub fn c() {}\n"),
        ("crates/c/target/debug/build/x.rs", "fn x() {}\n"),
    ]);
    let report = analyze(&files);
    let files = scanned(&report);
    for skipped in [
        "dist/bundle.js",
        "build/out.js",
        "coverage/lcov-report/prettify.js",
        "packages/a/dist/index.js",
        "packages/a/build/index.js",
        "crates/c/target/debug/build/x.rs",
    ] {
        assert!(
            !files.contains(&skipped.to_string()),
            "{skipped}: {files:?}"
        );
    }
    assert!(
        files.contains(&"crates/c/src/lib.rs".to_string()),
        "{files:?}"
    );
}

#[test]
fn declared_output_folders_stay_skipped_wherever_they_are() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"p","main":"./lib/build/index.js","files":["out/dist"]}"#,
        ),
        (
            "tsconfig.json",
            "{\n  // compiled output\n  \"compilerOptions\": { \"outDir\": \"./src/gen/build\" },\n}\n",
        ),
        ("src/index.ts", "export const a = 1;\n"),
        ("src/gen/build/index.js", "var a = 1;\n"),
        ("lib/build/index.js", "var b = 1;\n"),
        ("out/dist/index.js", "var c = 1;\n"),
        ("src/tasks/build/run.ts", "export const run = 1;\n"),
    ]);
    let files = scanned(&report);
    for skipped in [
        "src/gen/build/index.js",
        "lib/build/index.js",
        "out/dist/index.js",
    ] {
        assert!(
            !files.contains(&skipped.to_string()),
            "{skipped}: {files:?}"
        );
    }
    assert!(
        files.contains(&"src/tasks/build/run.ts".to_string()),
        "{files:?}"
    );
}

#[test]
fn gitignored_build_folder_inside_src_stays_skipped() {
    let report = analyze(&[
        ("package.json", r#"{"name":"p"}"#),
        (".gitignore", "src/generated/build/\n"),
        ("src/index.ts", "export const a = 1;\n"),
        ("src/generated/build/out.js", "var a = 1;\n"),
        ("src/core/build/run.ts", "export const run = 1;\n"),
    ]);
    let files = scanned(&report);
    assert!(
        !files.contains(&"src/generated/build/out.js".to_string()),
        "{files:?}"
    );
    assert!(
        files.contains(&"src/core/build/run.ts".to_string()),
        "{files:?}"
    );
}
