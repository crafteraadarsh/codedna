//! Task context and reports say what limited their answers, through the
//! real CLI:
//!
//! - `context --task` reports how many files matched against how many were
//!   shown, the task words nothing matched, files matched on their path only
//!   (too large to read, or prose and data), and that matching is lexical;
//! - every file picked for a task carries evidence: the line of its first
//!   mention of each word, or the import line that links it to a match;
//! - the Markdown briefing says the same in a short "Limits" line;
//! - `report` has an "Assumptions and limits" section, and each possibly
//!   unused file says whether it is unreachable or may still be loaded.

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
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn widget_repo(dir: &Path) {
    write(
        dir,
        &[
            (
                "src/index.ts",
                "import { render } from './widget';\nrender();\n",
            ),
            (
                "src/widget.ts",
                "import type { Props } from './props';\n\
                 import { paint } from './paint';\n\
                 export const render = (p?: Props) => paint(p);\n",
            ),
            ("src/props.ts", "export type Props = { size: number };\n"),
            ("src/paint.ts", "export const paint = (x: unknown) => x;\n"),
            ("src/widget-old.ts", "export const legacy = 1;\n"),
            ("docs/widget.md", "# Widget\n\nHow to render it.\n"),
        ],
    );
}

fn task(dir: &Path, words: &str) -> Value {
    let json = run(dir, &["context", ".", "--format", "json", "--task", words]);
    let pack: Value = serde_json::from_str(&json).unwrap();
    pack["task"].clone()
}

fn entry<'a>(task: &'a Value, file: &str) -> &'a Value {
    task["files"]
        .as_array()
        .unwrap()
        .iter()
        .chain(task["tests"].as_array().unwrap())
        .find(|f| f["file"].as_str().unwrap().trim_start_matches("./") == file)
        .unwrap_or_else(|| panic!("{file} is not listed: {task}"))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {value}"))
        .iter()
        // Paths may be shown as ./src/x.ts; import specifiers keep their ./
        .map(|v| {
            let v = v.as_str().unwrap();
            v.strip_prefix("./")
                .unwrap_or(v)
                .replace(" ./src/", " src/")
        })
        .collect()
}

#[test]
fn task_context_reports_its_limits() {
    let dir = tempfile::tempdir().unwrap();
    widget_repo(dir.path());
    // Over 512 KiB: matched on its path only.
    let big = format!(
        "export const render = 1;\n{}",
        "// padding\n".repeat(60_000)
    );
    write(dir.path(), &[("src/huge.ts", &big)]);

    let task = task(dir.path(), "render the widget zebra");
    let limits = &task["limits"];
    assert_eq!(limits["matching"], "lexical", "{task}");
    assert_eq!(strings(&limits["unmatched_terms"]), vec!["zebra"], "{task}");
    assert_eq!(limits["large_files"], 1, "{task}");
    assert_eq!(limits["path_only_files"], 1, "{task}");
    let shown = task["files"].as_array().unwrap().len() + task["tests"].as_array().unwrap().len();
    assert_eq!(limits["shown"], shown, "{task}");
    let candidates = limits["candidates"].as_u64().unwrap() as usize;
    assert!(candidates >= shown, "{task}");
    assert_eq!(limits["truncated"], candidates > shown, "{task}");
    assert!(limits["considered"].as_u64().unwrap() >= 7, "{task}");
    let notes = strings(&limits["notes"]).join(" ");
    assert!(notes.contains("wording, not meaning"), "{notes}");
    assert!(notes.contains("zebra"), "{notes}");
    assert!(notes.contains("512 KiB"), "{notes}");
}

#[test]
fn task_files_carry_evidence() {
    let dir = tempfile::tempdir().unwrap();
    widget_repo(dir.path());
    let task = task(dir.path(), "render the widget");

    // A direct match: the line of the first mention.
    let widget = strings(&entry(&task, "src/widget.ts")["evidence"]);
    assert!(
        widget.contains(&"src/widget.ts:3 mentions \"render\"".to_string()),
        "{widget:?}"
    );
    // A neighbour: the import line that links it to the match.
    let paint = strings(&entry(&task, "src/paint.ts")["evidence"]);
    assert!(
        paint.contains(&"src/widget.ts:2 imports './paint'".to_string()),
        "{paint:?}"
    );
    let index = strings(&entry(&task, "src/index.ts")["evidence"]);
    assert!(
        index.iter().any(|e| e.starts_with("src/index.ts:1")),
        "{index:?}"
    );
}

#[test]
fn markdown_briefing_says_its_limits() {
    let dir = tempfile::tempdir().unwrap();
    widget_repo(dir.path());
    let md = run(
        dir.path(),
        &["context", ".", "--task", "render the widget zebra"],
    );
    let section = &md[md.find("## Relevant to the task").expect(&md)..];
    assert!(section.contains("Limits:"), "{section}");
    assert!(section.contains("zebra"), "{section}");
}

#[test]
fn report_has_assumptions_and_why_per_unused_file() {
    let dir = tempfile::tempdir().unwrap();
    widget_repo(dir.path());
    let out = dir.path().join("report.md");
    run(dir.path(), &["report", ".", "-o", out.to_str().unwrap()]);
    let md = fs::read_to_string(&out).unwrap();
    let section = &md[md.find("## Assumptions and limits").expect(&md)..];
    assert!(section.contains("never run"), "{section}");
    assert!(section.contains("Entry points"), "{section}");
    let unused = &md[md.find("## Possibly unused files").expect(&md)..];
    let row = unused
        .lines()
        .find(|l| l.contains("widget-old.ts"))
        .expect(unused);
    assert!(row.contains("unreachable"), "{row}");
    assert!(row.contains("codedna explain"), "{row}");

    let html_out = dir.path().join("report.html");
    run(
        dir.path(),
        &[
            "report",
            ".",
            "--format",
            "html",
            "-o",
            html_out.to_str().unwrap(),
        ],
    );
    let html = fs::read_to_string(&html_out).unwrap();
    assert!(html.contains("Assumptions and limits"), "missing section");
    assert!(html.contains("unreachable"), "missing status");
}
