//! Dead code and circular imports read the kind and certainty of each
//! dependency, through the real CLI:
//!
//! - a file reached from a production entry through runtime imports is
//!   confirmed; one reached only through type imports, only from tests or
//!   only from tooling (stories, configs) is reported as such, with the path
//!   from the entry that reaches it;
//! - an unreachable file that a possibility points at (a shadowed
//!   `require`) is possibly loaded, and one that an unresolved import could
//!   name is unknown, while an unrelated unresolved import changes nothing;
//! - a cycle that needs a type-only import to close is reported as
//!   type-only, apart from runtime cycles.
//!
//! Every case comes with a contrasting one that keeps the old answer.

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

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(args)
        .arg("--no-cache")
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Windows reports `.\src\a.ts`; compare paths with forward slashes.
    // JSON escapes the separator, and other backslashes (`\"`) must stay.
    let out = String::from_utf8(output.stdout).unwrap().replace("\\\\", "/");
    if out.starts_with(['{', '[']) {
        out
    } else {
        out.replace(".\\", "./").replace('\\', "/")
    }
}

fn analyze(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), files);
    serde_json::from_str(&run(dir.path(), &["json", "--compact"])).unwrap()
}

fn clean(path: &Value) -> String {
    path.as_str()
        .unwrap()
        .trim_start_matches("./")
        .to_string()
}

fn dead(report: &Value) -> Vec<String> {
    report["dead_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(clean)
        .collect()
}

/// The `usage` entry for `file`, if it is not confirmed.
fn usage<'a>(report: &'a Value, file: &str) -> Option<&'a Value> {
    report["usage"]["files"]
        .as_array()
        .expect("usage.files is an array")
        .iter()
        .find(|u| clean(&u["file"]) == file)
}

fn path_of(entry: &Value) -> Vec<String> {
    entry["path"].as_array().unwrap().iter().map(clean).collect()
}

fn candidate<'a>(report: &'a Value, file: &str) -> &'a Value {
    report["dead_code_details"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| clean(&c["file"]) == file)
        .unwrap_or_else(|| panic!("{file} is not a dead-code candidate: {report}"))
}

#[test]
fn type_only_usage_is_kept_apart_from_runtime_usage() {
    let report = analyze(&[
        (
            "src/index.ts",
            "import type { Shape } from './shape';\n\
             import type { Color } from './color';\n\
             import { paint } from './color';\n\
             export const s: Shape = paint();\n",
        ),
        ("src/shape.ts", "export type Shape = { w: number };\n"),
        (
            "src/color.ts",
            "export type Color = string;\nexport const paint = () => ({ w: 1 });\n",
        ),
    ]);
    // Used for its types only: alive, but not confirmed.
    assert!(!dead(&report).contains(&"src/shape.ts".to_string()));
    let shape = usage(&report, "src/shape.ts").expect("shape.ts has a usage entry");
    assert_eq!(shape["usage"], "type_only");
    assert_eq!(path_of(shape), ["src/index.ts", "src/shape.ts"]);
    // Also imported at runtime: confirmed, so no entry.
    assert!(usage(&report, "src/color.ts").is_none(), "{report}");
    assert_eq!(report["usage"]["summary"]["type_only"], 1);
    assert_eq!(report["usage"]["summary"]["confirmed"], 2);
}

#[test]
fn test_and_tooling_usage_is_not_production_usage() {
    let report = analyze(&[
        ("src/index.ts", "import { used } from './used';\nused();\n"),
        ("src/used.ts", "export const used = () => 1;\n"),
        ("src/helper.ts", "export const helper = () => 2;\n"),
        (
            "src/used.test.ts",
            "import { used } from './used';\nimport { helper } from './helper';\nused(); helper();\n",
        ),
        ("src/Badge.tsx", "export const Badge = () => null;\n"),
        (
            "src/Badge.stories.tsx",
            "import { Badge } from './Badge';\nexport default { component: Badge };\n",
        ),
    ]);
    let dead = dead(&report);
    for file in ["src/helper.ts", "src/Badge.tsx", "src/used.ts"] {
        assert!(!dead.contains(&file.to_string()), "{file} is not dead");
    }
    let helper = usage(&report, "src/helper.ts").expect("helper.ts has a usage entry");
    assert_eq!(helper["usage"], "test_only");
    assert_eq!(path_of(helper), ["src/used.test.ts", "src/helper.ts"]);
    let badge = usage(&report, "src/Badge.tsx").expect("Badge.tsx has a usage entry");
    assert_eq!(badge["usage"], "tooling_only");
    assert_eq!(path_of(badge), ["src/Badge.stories.tsx", "src/Badge.tsx"]);
    // Imported by the app as well as the test: confirmed.
    assert!(usage(&report, "src/used.ts").is_none());
    // Entries themselves are reported by what loads them.
    assert_eq!(usage(&report, "src/used.test.ts").unwrap()["usage"], "test_only");
}

#[test]
fn possibilities_and_unresolved_imports_make_candidates_uncertain() {
    let report = analyze(&[
        (
            "src/index.ts",
            "const require = (p: string) => p;\n\
             require('./maybe');\n\
             import { draw } from './lib/chart';\n\
             import { gone } from './nothing-here';\n\
             draw(gone);\n",
        ),
        ("src/maybe.ts", "import { dep } from './maybe-dep';\nexport default dep;\n"),
        ("src/maybe-dep.ts", "export const dep = 1;\n"),
        ("src/lib/chart/chart.ts", "export const draw = (x: unknown) => x;\n"),
        ("src/orphan.ts", "export const orphan = 1;\n"),
    ]);
    let dead = dead(&report);
    for file in [
        "src/maybe.ts",
        "src/maybe-dep.ts",
        "src/lib/chart/chart.ts",
        "src/orphan.ts",
    ] {
        assert!(dead.contains(&file.to_string()), "{file} stays in dead_code");
    }

    let maybe = candidate(&report, "src/maybe.ts");
    assert_eq!(maybe["status"], "possible", "{maybe}");
    assert_eq!(maybe["confidence"], "low");
    assert!(
        maybe["conditions"][0].as_str().unwrap().contains("src/index.ts:2"),
        "{maybe}"
    );
    // Only reached through a possibly loaded file: possible too.
    let dep = candidate(&report, "src/maybe-dep.ts");
    assert_eq!(dep["status"], "possible", "{dep}");
    assert_eq!(dep["confidence"], "low");

    let chart = candidate(&report, "src/lib/chart/chart.ts");
    assert_eq!(chart["status"], "unknown", "{chart}");
    assert_eq!(chart["confidence"], "low");
    assert!(
        chart["conditions"][0].as_str().unwrap().contains("./lib/chart"),
        "{chart}"
    );

    // The unrelated unresolved import does not touch other candidates.
    let orphan = candidate(&report, "src/orphan.ts");
    assert_eq!(orphan["status"], "unreachable");
    assert_eq!(orphan["confidence"], "high");
    assert!(orphan.get("conditions").is_none(), "{orphan}");
    assert_eq!(report["usage"]["summary"]["unreachable"], 4);
}

#[test]
fn type_only_cycles_are_told_apart_from_runtime_cycles() {
    let files = [
        ("src/index.ts", "import './a';\nimport './c';\n"),
        ("src/a.ts", "import { b } from './b';\nexport const a = () => b;\n"),
        ("src/b.ts", "import { a } from './a';\nexport const b = () => a;\n"),
        (
            "src/c.ts",
            "import type { D } from './d';\nexport const c = (d: D) => d;\n",
        ),
        ("src/d.ts", "import { c } from './c';\nexport type D = typeof c;\n"),
    ];
    let report = analyze(&files);
    let cycles = report["circular_imports"].as_array().unwrap();
    assert_eq!(cycles.len(), 2, "one per group, as before");
    let details = report["cycle_details"].as_array().unwrap();
    assert_eq!(details.len(), 2);
    let kind_of = |file: &str| {
        details
            .iter()
            .find(|d| d["files"].as_array().unwrap().iter().any(|f| clean(f) == file))
            .map(|d| d["kind"].as_str().unwrap().to_string())
    };
    assert_eq!(kind_of("src/a.ts").as_deref(), Some("runtime"));
    assert_eq!(kind_of("src/c.ts").as_deref(), Some("type_only"));
    for (i, detail) in details.iter().enumerate() {
        assert_eq!(detail["files"], cycles[i], "details follow circular_imports");
    }

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &files);
    let text = run(dir.path(), &["cycles"]);
    assert!(text.contains("type-only"), "{text}");
    let json: Value = serde_json::from_str(&run(dir.path(), &["cycles", "--json"])).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 2);
}

#[test]
fn deadcode_text_names_uncertain_candidates() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "src/index.ts",
                "const require = (p: string) => p;\nrequire('./maybe');\n",
            ),
            ("src/maybe.ts", "export default 1;\n"),
        ],
    );
    let text = run(dir.path(), &["deadcode"]);
    assert!(text.contains("src/maybe.ts"), "{text}");
    assert!(text.contains("possibly loaded"), "{text}");
}
