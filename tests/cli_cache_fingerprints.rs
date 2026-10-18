//! The analysis cache seen through the real CLI. A warm run must report
//! exactly what a run with `--no-cache` reports, whatever changed in
//! between: file contents (including edits that keep the size and the
//! modification time), configuration files, resolver conditions, added and
//! removed files. A change must only recompute what it can affect.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde_json::Value;

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
}

/// Replace `from` with `to` in `path` (same length), then put the old
/// modification time back: neither the size nor the time changes.
fn edit_in_place(dir: &Path, path: &str, from: &str, to: &str) {
    assert_eq!(from.len(), to.len(), "edit must keep the size");
    let path = dir.join(path);
    let before = fs::metadata(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{from} not in {}", path.display());
    let edited = text.replacen(from, to, 1);
    fs::write(&path, &edited).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(before.modified().unwrap())
        .unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}

struct Run {
    report: Value,
    stats: Value,
}

/// `codedna json` for `dir`, with the cache in `cache` or off.
fn run(dir: &Path, cache: Option<&Path>) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codedna"));
    command.env_remove("CODEDNA_CACHE_TRUST_MTIME");
    match cache {
        Some(cache) => {
            command.env("CODEDNA_CACHE_DIR", cache);
            command.env("CODEDNA_CACHE_STATS", "1");
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
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "{stderr}");
    let text = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\\\\", "/");
    let report: Value = serde_json::from_str(&text).unwrap();
    let stats = stderr
        .lines()
        .find_map(|line| line.strip_prefix("codedna cache stats: "))
        .map(|json| serde_json::from_str(json).unwrap())
        .unwrap_or(Value::Null);
    Run { report, stats }
}

/// The parts of the report the cache feeds.
fn cached_parts(report: &Value) -> Value {
    serde_json::json!({
        "dependencies": report["dependencies"],
        "dependency_graph": report["dependency_graph"],
        "coverage": report["coverage"],
        "dead_code": report["dead_code"],
        "architecture": report["architecture"],
        "total_loc": report["total_loc"],
        "file_breakdown": report["file_breakdown"],
    })
}

/// A warm run in `cache` reports what a cache-off run reports.
fn assert_matches_fresh(dir: &Path, cache: &Path) -> Run {
    let warm = run(dir, Some(cache));
    let fresh = run(dir, None);
    assert_eq!(
        cached_parts(&warm.report),
        cached_parts(&fresh.report),
        "cache on and off disagree"
    );
    warm
}

fn edges<'a>(report: &'a Value, source: &str) -> Vec<&'a str> {
    let mut targets: Vec<&str> = report["dependency_graph"][source]
        .as_array()
        .map(|t| t.iter().map(|v| v.as_str().unwrap()).collect())
        .unwrap_or_default();
    targets.sort();
    targets
}

fn stat(run: &Run, kind: &str, what: &str) -> u64 {
    run.stats[kind][what]
        .as_u64()
        .unwrap_or_else(|| panic!("no {kind}.{what} in {}", run.stats))
}

#[test]
fn a_same_size_edit_with_the_old_timestamp_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "src/index.ts",
                "import { a } from \"./a\";\nconsole.log(a);\n",
            ),
            ("src/a.ts", "export const a = 1;\n"),
            ("src/b.ts", "export const a = 2;\n"),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    assert_eq!(edges(&cold.report, "./src/index.ts"), ["./src/a.ts"]);

    edit_in_place(dir.path(), "src/index.ts", "\"./a\"", "\"./b\"");
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(edges(&warm.report, "./src/index.ts"), ["./src/b.ts"]);
}

#[test]
fn a_same_size_tsconfig_paths_edit_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "tsconfig.json",
                "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"@x/*\":[\"src/one/*\"]}}}\n",
            ),
            (
                "src/index.ts",
                "import { v } from \"@x/v\";\nconsole.log(v);\n",
            ),
            ("src/one/v.ts", "export const v = 1;\n"),
            ("src/two/v.ts", "export const v = 2;\n"),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    assert_eq!(edges(&cold.report, "./src/index.ts"), ["./src/one/v.ts"]);

    edit_in_place(dir.path(), "tsconfig.json", "src/one/*", "src/two/*");
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(edges(&warm.report, "./src/index.ts"), ["./src/two/v.ts"]);
}

#[test]
fn a_same_size_condition_edit_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("package.json", "{\"private\":true,\"workspaces\":[\"packages/*\"]}\n"),
            (
                "packages/ui/package.json",
                "{\"name\":\"@acme/ui\",\"exports\":{\".\":{\"alpha\":\"./src/alpha.ts\",\"gamma\":\"./src/gamma.ts\",\"default\":\"./src/index.ts\"}}}\n",
            ),
            ("packages/ui/src/alpha.ts", "export const ui = 1;\n"),
            ("packages/ui/src/gamma.ts", "export const ui = 2;\n"),
            ("packages/ui/src/index.ts", "export const ui = 3;\n"),
            (
                "packages/app/package.json",
                "{\"name\":\"app\",\"dependencies\":{\"@acme/ui\":\"*\"}}\n",
            ),
            (
                "packages/app/tsconfig.json",
                "{\"compilerOptions\":{\"module\":\"esnext\",\"moduleResolution\":\"bundler\",\"customConditions\":[\"alpha\"]}}\n",
            ),
            (
                "packages/app/src/main.ts",
                "import { ui } from \"@acme/ui\";\nconsole.log(ui);\n",
            ),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    let before = edges(&cold.report, "./packages/app/src/main.ts");
    assert!(before.contains(&"./packages/ui/src/alpha.ts"), "{before:?}");

    edit_in_place(
        dir.path(),
        "packages/app/tsconfig.json",
        "[\"alpha\"]",
        "[\"gamma\"]",
    );
    let warm = assert_matches_fresh(dir.path(), cache.path());
    let after = edges(&warm.report, "./packages/app/src/main.ts");
    // TypeScript now takes gamma; alpha is only a labelled branch.
    assert!(after.contains(&"./packages/ui/src/gamma.ts"), "{after:?}");
}

#[test]
fn added_and_removed_files_are_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "src/index.ts",
                "import { x } from \"./x\";\nimport \"./gone\";\n",
            ),
            ("src/x.js", "export const x = 1;\n"),
            ("src/gone.ts", "export {};\n"),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    assert_eq!(
        edges(&cold.report, "./src/index.ts"),
        ["./src/gone.ts", "./src/x.js"]
    );

    // A TypeScript file next to the JavaScript one now wins; the removed
    // file no longer resolves.
    write(dir.path(), &[("src/x.ts", "export const x = 2;\n")]);
    fs::remove_file(dir.path().join("src/gone.ts")).unwrap();
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(edges(&warm.report, "./src/index.ts"), ["./src/x.ts"]);
}

#[test]
fn cold_and_warm_runs_match_a_fresh_run_across_languages() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "web/src/index.ts",
                "import { a } from \"./a\";\nconsole.log(a);\n",
            ),
            ("web/src/a.ts", "export const a = 1;\n"),
            ("web/src/unused.ts", "export const u = 1;\n"),
            ("api/app/__init__.py", ""),
            ("api/app/main.py", "from app import models\nprint(models)\n"),
            ("api/app/models.py", "X = 1\n"),
            (
                "core/Cargo.toml",
                "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
            ),
            ("core/src/main.rs", "mod util;\nfn main() { util::f(); }\n"),
            ("core/src/util.rs", "pub fn f() {}\n"),
        ],
    );
    let fresh = run(dir.path(), None);
    let cold = run(dir.path(), Some(cache.path()));
    let warm = run(dir.path(), Some(cache.path()));
    assert_eq!(cached_parts(&cold.report), cached_parts(&fresh.report));
    assert_eq!(cached_parts(&warm.report), cached_parts(&fresh.report));
    assert_eq!(stat(&warm, "imports", "computed"), 0, "{}", warm.stats);
    assert_eq!(stat(&warm, "scan", "computed"), 0, "{}", warm.stats);
    assert!(stat(&warm, "imports", "reused") > 0, "{}", warm.stats);
}

#[test]
fn a_tsconfig_change_recomputes_only_its_package() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let tsconfig =
        "{\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"~/*\":[\"src/one/*\"]}}}\n";
    write(
        dir.path(),
        &[
            (
                "package.json",
                "{\"private\":true,\"workspaces\":[\"packages/*\"]}\n",
            ),
            ("packages/a/package.json", "{\"name\":\"a\"}\n"),
            ("packages/a/tsconfig.json", tsconfig),
            (
                "packages/a/src/index.ts",
                "import { v } from \"~/v\";\nconsole.log(v);\n",
            ),
            ("packages/a/src/other.ts", "import \"./index\";\n"),
            ("packages/a/src/one/v.ts", "export const v = 1;\n"),
            ("packages/a/src/two/v.ts", "export const v = 2;\n"),
            ("packages/b/package.json", "{\"name\":\"b\"}\n"),
            ("packages/b/tsconfig.json", tsconfig),
            (
                "packages/b/src/index.ts",
                "import { v } from \"~/v\";\nconsole.log(v);\n",
            ),
            ("packages/b/src/other.ts", "import \"./index\";\n"),
            ("packages/b/src/one/v.ts", "export const v = 1;\n"),
            ("packages/b/src/two/v.ts", "export const v = 2;\n"),
        ],
    );
    run(dir.path(), Some(cache.path()));

    edit_in_place(
        dir.path(),
        "packages/a/tsconfig.json",
        "src/one/*",
        "src/two/*",
    );
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(
        edges(&warm.report, "./packages/a/src/index.ts"),
        ["./packages/a/src/two/v.ts"]
    );
    assert_eq!(
        edges(&warm.report, "./packages/b/src/index.ts"),
        ["./packages/b/src/one/v.ts"]
    );
    // Only package a's four source files depend on its tsconfig; nothing
    // needs parsing again because no source file changed.
    assert_eq!(stat(&warm, "imports", "computed"), 4, "{}", warm.stats);
    assert_eq!(stat(&warm, "imports", "reused"), 4, "{}", warm.stats);
    assert_eq!(stat(&warm, "scan", "computed"), 0, "{}", warm.stats);
}

#[test]
fn trusting_timestamps_is_an_explicit_faster_mode() {
    // With CODEDNA_CACHE_TRUST_MTIME the cache reuses a file whose size and
    // time are unchanged without reading it: a same-size edit that keeps
    // the old time is missed. That is the documented tradeoff.
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("src/index.ts", "import { a } from \"./a\";\n"),
            ("src/a.ts", "export const a = 1;\n"),
            ("src/b.ts", "export const a = 2;\n"),
        ],
    );
    let fast = |dir: &Path| {
        let output = Command::new(env!("CARGO_BIN_EXE_codedna"))
            .env("CODEDNA_CACHE_DIR", cache.path())
            .env("CODEDNA_CACHE_TRUST_MTIME", "1")
            .env_remove("CODEDNA_NO_CACHE")
            .args(["json", "--compact"])
            .arg(dir)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)
            .unwrap()
            .replace("\\\\", "/");
        serde_json::from_str::<Value>(&text).unwrap()
    };
    fast(dir.path());
    edit_in_place(dir.path(), "src/index.ts", "\"./a\"", "\"./b\"");
    assert_eq!(edges(&fast(dir.path()), "./src/index.ts"), ["./src/a.ts"]);
    // The default mode reads the bytes and sees the edit.
    let strict = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(edges(&strict.report, "./src/index.ts"), ["./src/b.ts"]);
}

fn certainty<'a>(report: &'a Value, source: &str) -> Vec<&'a str> {
    report["dependencies"]["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["source"] == source)
        .map(|f| f["certainty"].as_str().unwrap())
        .collect()
}

#[test]
fn a_resolver_mode_change_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("package.json", "{\"name\":\"app\",\"type\":\"module\"}\n"),
            (
                "tsconfig.json",
                "{\"compilerOptions\":{\"module\":\"esnext\",\"moduleResolution\":\"bundler\"}}\n",
            ),
            ("src/util.ts", "export const u = 1;\n"),
            (
                "src/main.ts",
                "import { u } from \"./util\";\nconsole.log(u);\n",
            ),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    assert_eq!(certainty(&cold.report, "./src/main.ts"), ["direct"]);

    // Under nodenext an extensionless relative ES import is not how Node
    // resolves it: the edge stays, but only inferred.
    write(
        dir.path(),
        &[(
            "tsconfig.json",
            "{\"compilerOptions\":{\"module\":\"nodenext\",\"moduleResolution\":\"nodenext\"}}\n",
        )],
    );
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(certainty(&warm.report, "./src/main.ts"), ["inferred"]);
}

#[test]
fn a_framework_config_added_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("package.json", "{\"name\":\"root\",\"private\":true}\n"),
            ("src/lib/util.ts", "export const a = 1;\n"),
            ("apps/web/src/lib/util.ts", "export const b = 1;\n"),
            (
                "apps/web/src/routes/page.ts",
                "import { b } from \"$lib/util\";\nconsole.log(b);\n",
            ),
        ],
    );
    let cold = run(dir.path(), Some(cache.path()));
    assert_eq!(
        edges(&cold.report, "./apps/web/src/routes/page.ts"),
        ["./src/lib/util.ts"]
    );

    // A SvelteKit config pins `$lib` to its own `src/lib`.
    write(
        dir.path(),
        &[("apps/web/svelte.config.js", "export default {};\n")],
    );
    let warm = assert_matches_fresh(dir.path(), cache.path());
    assert_eq!(
        edges(&warm.report, "./apps/web/src/routes/page.ts"),
        ["./apps/web/src/lib/util.ts"]
    );
}
