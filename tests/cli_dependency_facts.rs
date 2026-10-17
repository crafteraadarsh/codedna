//! Typed dependency facts seen through the real CLI: every dependency the
//! analysis finds is reported with its kind, how it resolved, how certain
//! it is and where it was found, and `dependency_graph` is exactly the
//! resolved facts.

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
        // Old enough for the cache to keep it (it skips files changed in
        // the last moments of a run).
        let old = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
}

/// Write `files` into a fresh directory and return `codedna json` for it.
fn analyze(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), files);
    run(dir.path(), None)
}

/// `codedna json` for `dir`, with the cache in `cache` or off.
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
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_graph_is_projection(&report);
    report
}

fn facts(report: &Value) -> &Vec<Value> {
    report["dependencies"]["facts"]
        .as_array()
        .expect("dependencies.facts is an array")
}

/// The one fact from `source` for `specifier`.
fn fact<'a>(report: &'a Value, source: &str, specifier: &str) -> &'a Value {
    let found: Vec<&Value> = facts(report)
        .iter()
        .filter(|f| f["source"] == source && f["specifier"] == specifier)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one fact {source} -> {specifier} in {:#}",
        report["dependencies"]
    );
    found[0]
}

/// `dependency_graph` holds exactly the targets of resolved facts, per
/// source, and a node for every source file.
fn assert_graph_is_projection(report: &Value) {
    let graph = report["dependency_graph"].as_object().unwrap();
    for (source, targets) in graph {
        for target in targets.as_array().unwrap() {
            let backed = facts(report).iter().any(|f| {
                f["source"] == source.as_str()
                    && f["resolution"] == "resolved"
                    && f["targets"].as_array().unwrap().contains(target)
            });
            assert!(backed, "edge {source} -> {target} has no resolved fact");
        }
    }
    for f in facts(report) {
        let source = f["source"].as_str().unwrap();
        let edges = graph
            .get(source)
            .unwrap_or_else(|| panic!("{source} is not a graph node"));
        assert_eq!(f["evidence"]["file"], source);
        let targets = f["targets"].as_array().unwrap();
        if f["resolution"] == "resolved" {
            for target in targets {
                assert!(
                    edges.as_array().unwrap().contains(target),
                    "resolved fact {source} -> {target} missing from the graph"
                );
            }
        } else {
            assert!(targets.is_empty(), "{f} has targets but did not resolve");
        }
    }
}

const INDEX_TS: &str = "import { a } from \"./a\";\n\
import type { T } from \"./types\";\n\
export { b } from \"./b\";\n\
import \"./side\";\n\
const c = require(\"./c\");\n\
const lazy = () => import(\"./lazy\");\n\
import React from \"react\";\n\
import { gone } from \"./missing\";\n\
const page = (name: string) => import(`./pages/${name}.ts`);\n";

fn js_project() -> Vec<(&'static str, &'static str)> {
    vec![
        ("src/index.ts", INDEX_TS),
        ("src/a.ts", "export const a = 1;\n"),
        ("src/types.ts", "export type T = number;\n"),
        ("src/b.ts", "export const b = 1;\n"),
        ("src/side.ts", "console.log(1);\n"),
        ("src/c.ts", "module.exports = 1;\n"),
        ("src/lazy.ts", "export default 1;\n"),
    ]
}

#[test]
fn js_imports_are_typed_facts_with_evidence() {
    let report = analyze(&js_project());
    assert_eq!(report["dependencies"]["rules_version"], 5);
    let src = "./src/index.ts";

    let a = fact(&report, src, "./a");
    assert_eq!(a["kind"], "runtime_import");
    assert_eq!(a["resolution"], "resolved");
    assert_eq!(a["certainty"], "direct");
    assert_eq!(a["targets"], json!(["./src/a.ts"]));
    assert_eq!(a["evidence"]["line"], 1);
    assert_eq!(a["evidence"]["rule"], "js.import");
    assert!(a.get("condition").is_none() || a["condition"].is_null());

    let types = fact(&report, src, "./types");
    assert_eq!(types["kind"], "type_only_import");
    assert_eq!(types["evidence"]["line"], 2);

    let b = fact(&report, src, "./b");
    assert_eq!(b["kind"], "re_export");
    assert_eq!(b["evidence"]["rule"], "js.export_from");

    assert_eq!(
        fact(&report, src, "./side")["evidence"]["rule"],
        "js.side_effect_import"
    );
    assert_eq!(fact(&report, src, "./c")["evidence"]["rule"], "js.require");

    let lazy = fact(&report, src, "./lazy");
    assert_eq!(lazy["kind"], "runtime_import");
    assert_eq!(lazy["condition"], "dynamic");
    assert_eq!(lazy["evidence"]["rule"], "js.dynamic_import");

    let react = fact(&report, src, "react");
    assert_eq!(react["resolution"], "external");
    assert_eq!(react["targets"], json!([]));

    // An import of a missing file survives as an unresolved fact.
    let gone = fact(&report, src, "./missing");
    assert_eq!(gone["resolution"], "unresolved");
    assert_eq!(gone["kind"], "runtime_import");
    assert_eq!(gone["evidence"]["line"], 8);

    let page = fact(&report, src, "./pages/${name}.ts");
    assert_eq!(page["kind"], "possible_dynamic_load");
    assert_eq!(page["resolution"], "unsupported");
    assert_eq!(page["certainty"], "possibility");

    // The graph is unchanged: the resolved imports in source order.
    assert_eq!(
        report["dependency_graph"][src],
        json!([
            "./src/a.ts",
            "./src/types.ts",
            "./src/b.ts",
            "./src/side.ts",
            "./src/c.ts",
            "./src/lazy.ts"
        ])
    );
}

#[test]
fn evidence_spans_point_at_the_specifier() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &js_project());
    let report = run(dir.path(), None);
    let content = fs::read(dir.path().join("src/index.ts")).unwrap();
    for specifier in ["./a", "./missing", "react"] {
        let span = &fact(&report, "./src/index.ts", specifier)["evidence"]["span"];
        let (start, end) = (
            span[0].as_u64().unwrap() as usize,
            span[1].as_u64().unwrap() as usize,
        );
        assert_eq!(&content[start..end], specifier.as_bytes());
    }
}

#[test]
fn runtime_loaders_are_worker_process_and_resource_facts() {
    let report = analyze(&[
        (
            "src/index.ts",
            "import { spawn } from \"node:child_process\";\n\
             const w = new Worker(new URL(\"./bundled.ts\", import.meta.url));\n\
             const plain = new Worker(\"./plain.js\");\n\
             spawn(\"tsx\", [\"./job.ts\"]);\n\
             console.log(\"./logged.ts\");\n",
        ),
        ("src/bundled.ts", "export {};\n"),
        ("src/plain.js", "export {};\n"),
        ("src/job.ts", "export {};\n"),
        ("src/logged.ts", "export {};\n"),
    ]);
    let src = "./src/index.ts";
    let bundled = fact(&report, src, "./bundled.ts");
    assert_eq!(bundled["kind"], "resource_reference");
    assert_eq!(bundled["certainty"], "inferred");
    assert_eq!(bundled["evidence"]["line"], 2);
    let plain = fact(&report, src, "./plain.js");
    assert_eq!(plain["kind"], "worker_entry");
    assert_eq!(plain["targets"], json!(["./src/plain.js"]));
    let job = fact(&report, src, "./job.ts");
    assert_eq!(job["kind"], "process_entry");
    assert_eq!(job["evidence"]["rule"], "js.process");
    assert_eq!(job["evidence"]["line"], 4);
    // A path in a log message is not a dependency.
    assert!(!facts(&report)
        .iter()
        .any(|f| f["specifier"] == "./logged.ts"));
}

#[test]
fn python_imports_and_configured_entries_are_facts() {
    let report = analyze(&[
        ("shop/__init__.py", ""),
        ("shop/apps.py", "from django.apps import AppConfig\n"),
        (
            "shop/models.py",
            "from .helpers import slug\nimport requests\nfrom .gone import x\n",
        ),
        ("shop/helpers.py", "def slug(): pass\n"),
        ("proj/__init__.py", ""),
        ("proj/settings.py", "ROOT_URLCONF = \"proj.urls\"\n"),
        ("proj/urls.py", "urlpatterns = []\n"),
    ]);
    let models = "./shop/models.py";
    let helpers = fact(&report, models, ".helpers");
    assert_eq!(helpers["kind"], "runtime_import");
    assert_eq!(helpers["resolution"], "resolved");
    assert_eq!(helpers["targets"], json!(["./shop/helpers.py"]));
    assert_eq!(helpers["evidence"]["rule"], "python.from_import");
    assert_eq!(fact(&report, models, "requests")["resolution"], "external");
    let gone = fact(&report, models, ".gone");
    assert_eq!(gone["resolution"], "unresolved");
    assert_eq!(gone["evidence"]["line"], 3);

    let urls = fact(&report, "./proj/settings.py", "proj.urls");
    assert_eq!(urls["kind"], "configured_entry");
    assert_eq!(urls["certainty"], "inferred");
    // Python runs the package's `__init__.py` before the module.
    assert_eq!(
        urls["targets"],
        json!(["./proj/__init__.py", "./proj/urls.py"])
    );

    let app = facts(&report)
        .iter()
        .find(|f| {
            f["source"] == "./shop/__init__.py" && f["targets"] == json!(["./shop/models.py"])
        })
        .expect("Django app module fact");
    assert_eq!(app["kind"], "configured_entry");
    assert_eq!(app["evidence"]["rule"], "python.django_app_module");
}

#[test]
fn rust_mod_declarations_are_declaration_facts() {
    let report = analyze(&[
        ("src/main.rs", "mod a;\nmod missing;\nfn main() {}\n"),
        ("src/a.rs", "pub fn a() {}\n"),
    ]);
    let a = fact(&report, "./src/main.rs", "a");
    assert_eq!(a["kind"], "runtime_import");
    assert_eq!(a["certainty"], "declaration");
    assert_eq!(a["resolution"], "resolved");
    assert_eq!(a["evidence"]["rule"], "rust.mod");
    assert_eq!(a["evidence"]["span"], json!([4, 5]));
    let missing = fact(&report, "./src/main.rs", "missing");
    assert_eq!(missing["resolution"], "unresolved");
    assert_eq!(missing["evidence"]["line"], 2);
}

#[test]
fn facts_are_sorted_and_stable() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &js_project());
    write(
        dir.path(),
        &[(
            "src/a.ts",
            "import { b } from \"./b\";\nexport const a = 1;\n",
        )],
    );
    let first = run(dir.path(), None);
    let second = run(dir.path(), None);
    assert_eq!(first["dependencies"], second["dependencies"]);
    let keys: Vec<(String, u64)> = facts(&first)
        .iter()
        .map(|f| {
            (
                f["source"].as_str().unwrap().to_string(),
                f["evidence"]["line"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn facts_match_with_the_cache_on_and_off_and_follow_edits() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(dir.path(), &js_project());

    let off = run(dir.path(), None);
    let cold = run(dir.path(), Some(cache.path()));
    let warm = run(dir.path(), Some(cache.path()));
    assert_eq!(off["dependencies"], cold["dependencies"]);
    assert_eq!(off["dependencies"], warm["dependencies"]);
    assert_eq!(off["dependency_graph"], warm["dependency_graph"]);
    assert_eq!(off["coverage"], warm["coverage"]);

    // An edited file is read again, not served from the cache.
    write(
        dir.path(),
        &[(
            "src/a.ts",
            "import { b } from \"./b\";\nexport const a = 22;\n",
        )],
    );
    let edited = run(dir.path(), Some(cache.path()));
    let b = fact(&edited, "./src/a.ts", "./b");
    assert_eq!(b["resolution"], "resolved");
    assert_eq!(
        run(dir.path(), None)["dependencies"],
        edited["dependencies"]
    );
}
