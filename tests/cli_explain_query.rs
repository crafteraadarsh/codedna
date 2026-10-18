//! `codedna explain` and `codedna query` show why CodeDna says what it says,
//! through the real CLI:
//!
//! - a used file comes with the entry point that reaches it, why that file
//!   is an entry point, and the import behind every step of the path, each
//!   with its file, line and rule; a file reached only through a type import
//!   shows that edge as type-only;
//! - an unreachable file comes with its dead-code finding, a stable id that
//!   `explain` accepts as well, and the unresolved import that may name it;
//! - a file loaded because a manifest names it cites the manifest line;
//! - a file in a language whose imports are not followed says so instead of
//!   claiming it is unused;
//! - `query` answers references, imports, shortest paths, entry points and
//!   unresolved imports, each with evidence; a path that does not exist is
//!   reported as not found, not invented;
//! - every answer carries provenance: analyzer, schema and rules versions,
//!   the active adapters and a snapshot hash that is stable across runs.

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

fn output(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(args)
        .arg("--no-cache")
        .arg(dir)
        .output()
        .unwrap()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let out = output(dir, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Windows reports `.\src\a.ts`; compare paths with forward slashes.
    let text = String::from_utf8(out.stdout).unwrap().replace("\\\\", "/");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn text(dir: &Path, args: &[&str]) -> String {
    let out = output(dir, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().replace('\\', "/")
}

fn clean(path: &Value) -> String {
    path.as_str()
        .unwrap_or_else(|| panic!("not a string: {path}"))
        .trim_start_matches("./")
        .to_string()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "src/index.ts",
                "import { a } from './a';\n\
                 import type { Shape } from './shape';\n\
                 import { gone } from './missing';\n\
                 import { o } from '@/lib/orphan';\n\
                 export const s: Shape = a();\n",
            ),
            ("src/a.ts", "import { b } from './b';\nexport const a = b;\n"),
            ("src/b.ts", "export const b = () => ({ w: 1 });\n"),
            ("src/shape.ts", "export type Shape = { w: number };\n"),
            ("src/lib/orphan.ts", "export const o = 1;\n"),
            ("src/stray.ts", "export const stray = 1;\n"),
            ("src/c.ts", "import { d } from './d';\nexport const c = d;\n"),
            ("src/d.ts", "import { c } from './c';\nexport const d = c;\n"),
            ("tools/gen.go", "package main\n\nfunc main() {}\n"),
        ],
    );
    dir
}

fn steps(explained: &Value) -> Vec<(String, String)> {
    explained["witness"]
        .as_array()
        .unwrap_or_else(|| panic!("no witness: {explained}"))
        .iter()
        .map(|s| (clean(&s["from"]), clean(&s["to"])))
        .collect()
}

#[test]
fn a_used_file_shows_its_root_and_the_evidence_for_each_step() {
    let dir = project();
    let v = json(dir.path(), &["explain", "src/b.ts", "--json"]);
    assert_eq!(v["id"], "file:./src/b.ts");
    assert_eq!(v["usage"], "confirmed", "{v}");
    assert_eq!(clean(&v["root"]["file"]), "src/index.ts");
    assert_eq!(v["root"]["role"], "production");
    assert!(
        v["root"]["reason"].as_str().unwrap().contains("entry point"),
        "{v}"
    );
    assert_eq!(
        steps(&v),
        [
            ("src/index.ts".to_string(), "src/a.ts".to_string()),
            ("src/a.ts".to_string(), "src/b.ts".to_string()),
        ]
    );
    let first = &v["witness"][0];
    assert_eq!(first["edge"], "runtime");
    assert_eq!(first["kind"], "runtime_import");
    assert_eq!(first["specifier"], "./a");
    assert_eq!(clean(&first["evidence"]["file"]), "src/index.ts");
    assert_eq!(first["evidence"]["line"], 1);
    assert_eq!(first["evidence"]["rule"], "js.import");
    assert_eq!(v["witness"][1]["evidence"]["line"], 1);
    assert!(v["dead_code"].is_null());
    // Who imports it, with evidence.
    assert_eq!(v["imported_by"]["total"], 1);
    assert_eq!(clean(&v["imported_by"]["facts"][0]["source"]), "src/a.ts");
    assert!(v["assumptions"].as_array().unwrap().len() >= 2, "{v}");
}

#[test]
fn a_type_only_file_shows_the_type_edge() {
    let dir = project();
    let v = json(dir.path(), &["explain", "./src/shape.ts", "--json"]);
    assert_eq!(v["usage"], "type_only");
    assert_eq!(
        steps(&v),
        [("src/index.ts".to_string(), "src/shape.ts".to_string())]
    );
    assert_eq!(v["witness"][0]["edge"], "type");
    assert_eq!(v["witness"][0]["kind"], "type_only_import");
    assert_eq!(v["witness"][0]["evidence"]["line"], 2);
}

#[test]
fn an_unreachable_file_shows_its_finding_and_what_may_still_load_it() {
    let dir = project();
    let v = json(dir.path(), &["explain", "src/lib/orphan.ts", "--json"]);
    assert_eq!(v["usage"], "unreachable");
    assert!(v["root"].is_null());
    assert_eq!(v["witness"].as_array().unwrap().len(), 0);
    let finding = &v["dead_code"];
    assert_eq!(finding["id"], "dead:./src/lib/orphan.ts");
    assert_eq!(finding["status"], "unknown", "{v}");
    assert!(
        finding["conditions"][0]
            .as_str()
            .unwrap()
            .contains("@/lib/orphan"),
        "{v}"
    );
    // The finding id explains the same file.
    let by_id = json(dir.path(), &["explain", "dead:./src/lib/orphan.ts", "--json"]);
    assert_eq!(by_id["dead_code"], v["dead_code"]);
    assert_eq!(by_id["id"], "dead:./src/lib/orphan.ts");

    // Contrast: a stray file nothing may name stays plainly unreachable.
    let stray = json(dir.path(), &["explain", "src/stray.ts", "--json"]);
    assert_eq!(stray["dead_code"]["status"], "unreachable");
    assert!(stray["dead_code"]["conditions"].is_null() || stray["dead_code"]["conditions"] == Value::Array(vec![]));
}

#[test]
fn a_cycle_finding_shows_each_edge_that_closes_it() {
    let dir = project();
    let v = json(dir.path(), &["explain", "src/c.ts", "--json"]);
    let id = v["cycle"].as_str().expect("c.ts is in a cycle").to_string();
    assert_eq!(id, "cycle:./src/c.ts");
    let cycle = json(dir.path(), &["explain", &id, "--json"]);
    assert_eq!(cycle["type"], "cycle");
    assert_eq!(cycle["kind"], "runtime");
    let edges = cycle["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2, "{cycle}");
    for edge in edges {
        assert_eq!(edge["evidence"]["line"], 1);
        assert_eq!(edge["evidence"]["rule"], "js.import");
    }
}

#[test]
fn a_manifest_entry_cites_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "package.json",
                "{\n  \"name\": \"tool\",\n  \"main\": \"lib/start.js\"\n}\n",
            ),
            ("lib/start.js", "const h = require('./helper');\nh();\n"),
            ("lib/helper.js", "module.exports = () => 1;\n"),
        ],
    );
    let v = json(dir.path(), &["explain", "lib/helper.js", "--json"]);
    assert_eq!(v["usage"], "confirmed", "{v}");
    assert_eq!(clean(&v["root"]["file"]), "lib/start.js");
    assert_eq!(clean(&v["root"]["evidence"]["file"]), "package.json");
    assert_eq!(v["root"]["evidence"]["line"], 3);
    assert!(
        v["root"]["evidence"]["rule"]
            .as_str()
            .unwrap()
            .starts_with("node."),
        "{v}"
    );
    let adapters = v["provenance"]["adapters"].as_array().unwrap();
    assert!(
        adapters.iter().any(|a| a.as_str().unwrap().starts_with("node@")),
        "{v}"
    );
}

#[test]
fn a_file_whose_language_is_not_followed_is_not_called_unused() {
    let dir = project();
    let v = json(dir.path(), &["explain", "tools/gen.go", "--json"]);
    assert_eq!(v["analyzed"], false);
    assert!(v["usage"].is_null(), "{v}");
    assert!(
        v["assumptions"][0].as_str().unwrap().contains("Go"),
        "{v}"
    );
}

#[test]
fn an_unknown_target_is_an_error() {
    let dir = project();
    let out = output(dir.path(), &["explain", "src/nope.ts"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("src/nope.ts"));
}

#[test]
fn explain_reads_as_text() {
    let dir = project();
    let out = text(dir.path(), &["explain", "src/b.ts"]);
    assert!(out.contains("src/index.ts:1"), "{out}");
    assert!(out.contains("Assumptions"), "{out}");
}

#[test]
fn queries_answer_with_evidence() {
    let dir = project();
    let refs = json(dir.path(), &["query", "references", "src/a.ts", "--json"]);
    assert_eq!(refs["total"], 1);
    assert_eq!(clean(&refs["facts"][0]["source"]), "src/index.ts");
    assert_eq!(refs["facts"][0]["evidence"]["line"], 1);

    let imports = json(dir.path(), &["query", "imports", "src/index.ts", "--json"]);
    let specifiers: Vec<&str> = imports["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["specifier"].as_str().unwrap())
        .collect();
    assert_eq!(specifiers, ["./a", "./shape", "./missing", "@/lib/orphan"]);

    let path = json(
        dir.path(),
        &["query", "path", "src/index.ts", "src/b.ts", "--json"],
    );
    assert_eq!(path["found"], true);
    assert_eq!(path["steps"].as_array().unwrap().len(), 2);
    assert_eq!(path["steps"][1]["evidence"]["line"], 1);
    // Contrast: nothing leads back up, and nothing is invented.
    let back = json(
        dir.path(),
        &["query", "path", "src/b.ts", "src/index.ts", "--json"],
    );
    assert_eq!(back["found"], false);
    assert_eq!(back["steps"].as_array().unwrap().len(), 0);

    let entries = json(dir.path(), &["query", "entrypoints", "--json"]);
    let index = entries["entrypoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| clean(&e["file"]) == "src/index.ts")
        .expect("index.ts is an entry point");
    assert_eq!(index["role"], "production");

    let unresolved = json(dir.path(), &["query", "unresolved", "--json"]);
    let found: Vec<(String, u64)> = unresolved["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["specifier"].as_str().unwrap().to_string(),
                f["evidence"]["line"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(found.contains(&("./missing".to_string(), 3)), "{unresolved}");
    assert!(found.contains(&("@/lib/orphan".to_string(), 4)), "{unresolved}");
}

#[test]
fn every_answer_carries_stable_provenance() {
    let dir = project();
    let one = json(dir.path(), &["explain", "src/b.ts", "--json"]);
    let two = json(dir.path(), &["query", "entrypoints", "--json"]);
    for v in [&one, &two] {
        let p = &v["provenance"];
        assert_eq!(p["analyzer"]["name"], "codedna");
        assert_eq!(p["schema_version"], 1);
        assert!(p["rules_version"].as_u64().unwrap() >= 5);
        assert_eq!(p["analysis"], "static");
        assert!(p["revision"].is_null(), "not a git repository: {p}");
        assert_eq!(p["snapshot"].as_str().unwrap().len(), 16);
    }
    assert_eq!(one["provenance"], two["provenance"]);
    // A change to the code changes the snapshot.
    fs::write(dir.path().join("src/b.ts"), "export const b = () => 2;\n").unwrap();
    let three = json(dir.path(), &["explain", "src/b.ts", "--json"]);
    assert_ne!(one["provenance"]["snapshot"], three["provenance"]["snapshot"]);
}
