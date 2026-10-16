//! Dependency edges seen through the real CLI: a file is reachable only
//! when code loads it, not when its name shows up in a string or comment.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

/// Write `files` into a fresh directory and return `codedna json` for it.
fn analyze(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    run(dir.path())
}

fn run(dir: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(["--no-cache", "json", "--compact"])
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

#[test]
fn log_strings_do_not_make_files_reachable() {
    let report = analyze(&[
        ("src/index.ts", "console.log(\"./unused.ts\");\n"),
        ("src/unused.ts", "export const unused = 1;\n"),
    ]);
    assert_eq!(report["dead_code"], json!(["./src/unused.ts"]));
    assert_eq!(report["dependency_graph"]["./src/index.ts"], json!([]));
}

#[test]
fn comments_and_other_strings_do_not_make_files_reachable() {
    let report = analyze(&[
        (
            "src/index.ts",
            "// new Worker(new URL(\"./old.ts\", import.meta.url));\n\
             /* spawn(\"tsx\", [\"./old.ts\"]) */\n\
             const help = `run with: tsx ./helper.ts`;\n\
             throw new Error(\"missing ./config.ts\");\n",
        ),
        ("src/old.ts", "export {};\n"),
        ("src/helper.ts", "export {};\n"),
        ("src/config.ts", "export {};\n"),
    ]);
    assert_eq!(
        dead_code(&report),
        ["./src/config.ts", "./src/helper.ts", "./src/old.ts"]
    );
}

#[test]
fn static_imports_still_count() {
    let report = analyze(&[
        (
            "src/index.ts",
            "import { used } from \"./used\";\nused();\n",
        ),
        ("src/used.ts", "export function used() {}\n"),
        ("src/orphan.ts", "export {};\n"),
    ]);
    assert_eq!(dead_code(&report), ["./src/orphan.ts"]);
}

#[test]
fn workers_loaded_by_url_count() {
    let report = analyze(&[
        (
            "src/index.ts",
            "const worker = new Worker(\n  new URL(\"./thread.ts\", import.meta.url),\n  { type: \"module\" },\n);\n\
             console.log(\"see ./orphan.ts\");\n",
        ),
        ("src/thread.ts", "export {};\n"),
        ("src/orphan.ts", "export {};\n"),
    ]);
    assert_eq!(dead_code(&report), ["./src/orphan.ts"]);
}

#[test]
fn imported_process_loaders_count_across_lines() {
    let report = analyze(&[
        (
            "src/index.ts",
            "import { spawn } from \"node:child_process\";\n\
             import * as cp from \"child_process\";\n\
             spawn(process.execPath, [\n  \"--import\",\n  \"tsx\",\n  \"worker.ts\",\n]);\n\
             cp.fork(\"./job.js\");\n\
             console.log(\"also ./orphan.ts\");\n",
        ),
        ("src/worker.ts", "export {};\n"),
        ("src/job.js", "module.exports = {};\n"),
        ("src/orphan.ts", "export {};\n"),
    ]);
    assert_eq!(dead_code(&report), ["./src/orphan.ts"]);
}

#[test]
fn local_functions_named_like_loaders_do_not_count() {
    let report = analyze(&[
        (
            "src/index.ts",
            "function spawn(message: string, files: string[]) { console.log(message, files); }\n\
             class Worker { constructor(public name: string) {} }\n\
             spawn(\"starting\", [\"./job.ts\"]);\n\
             new Worker(\"./thread.ts\");\n\
             /x.ts/.exec(\"./other.ts\");\n",
        ),
        ("src/job.ts", "export {};\n"),
        ("src/thread.ts", "export {};\n"),
        ("src/other.ts", "export {};\n"),
    ]);
    assert_eq!(
        dead_code(&report),
        ["./src/job.ts", "./src/other.ts", "./src/thread.ts"]
    );
}
