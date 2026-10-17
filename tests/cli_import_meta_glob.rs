//! Vite's `import.meta.glob`: a literal pattern is expanded against the
//! scanned files and each match is reached; a pattern only known at runtime
//! is a possibility with no edge. Every case comes as a pair: the files the
//! glob names, and the look-alikes it must not reach.

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
        .replace("\\\\", "/")
        .replace(".\\", "./");
    serde_json::from_str(&text).unwrap()
}

fn clean(path: &str) -> String {
    path.trim_start_matches("./").to_string()
}

fn fact<'a>(report: &'a Value, specifier: &str) -> &'a Value {
    let found: Vec<&Value> = report["dependencies"]["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["specifier"] == specifier)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one fact for {specifier} in {:#}",
        report["dependencies"]
    );
    found[0]
}

fn targets(f: &Value) -> Vec<String> {
    let mut found: Vec<String> = f["targets"]
        .as_array()
        .map(|t| t.iter().map(|p| clean(p.as_str().unwrap())).collect())
        .unwrap_or_default();
    found.sort();
    found
}

fn edges(report: &Value, source: &str) -> Vec<String> {
    let mut found: Vec<String> = report["dependency_graph"][format!("./{source}")]
        .as_array()
        .unwrap_or_else(|| panic!("{source} is not a node: {:#}", report["dependency_graph"]))
        .iter()
        .map(|t| clean(t.as_str().unwrap()))
        .collect();
    found.sort();
    found
}

fn dead(report: &Value) -> Vec<String> {
    report["dead_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| clean(f.as_str().unwrap()))
        .collect()
}

fn rule(f: &Value) -> &str {
    f["evidence"]["rule"].as_str().unwrap()
}

const MAIN: &str = "src/main.ts";
// Fixtures avoid `pages/`: files there are file routes, live without a glob.

/// A Vite app whose entry is `src/main.ts`, plus `files`.
fn app(main: &str, files: &[(&str, &str)]) -> Value {
    let mut all = vec![
        (
            "package.json",
            r#"{"name":"app","type":"module","main":"src/main.ts","devDependencies":{"vite":"6"}}"#,
        ),
        ("vite.config.ts", "export default {};\n"),
        (MAIN, main),
    ];
    all.extend_from_slice(files);
    analyze(&all)
}

#[test]
fn reports_the_new_rules_version() {
    let report = analyze(&[("a.ts", "export {};\n")]);
    assert_eq!(report["dependencies"]["rules_version"], 5);
}

#[test]
fn a_literal_glob_reaches_every_matching_file_and_nothing_else() {
    let report = app(
        "const cards = import.meta.glob('./cards/*.ts');\nexport default cards;\n",
        &[
            ("src/cards/home.ts", "export {};\n"),
            ("src/cards/about.ts", "export {};\n"),
            // Wrong extension, nested folder, sibling folder: not matched.
            ("src/cards/style.js", "export {};\n"),
            ("src/cards/deep/inner.ts", "export {};\n"),
            ("src/other/home.ts", "export {};\n"),
        ],
    );
    let glob = fact(&report, "./cards/*.ts");
    assert_eq!(glob["kind"], "runtime_import");
    assert_eq!(glob["certainty"], "direct");
    assert_eq!(glob["resolution"], "resolved");
    assert_eq!(rule(glob), "js.import_meta_glob");
    assert_eq!(targets(glob), ["src/cards/about.ts", "src/cards/home.ts"]);
    assert_eq!(
        edges(&report, MAIN),
        ["src/cards/about.ts", "src/cards/home.ts"]
    );

    let dead = dead(&report);
    assert!(!dead.contains(&"src/cards/home.ts".to_string()), "{dead:?}");
    for file in [
        "src/cards/style.js",
        "src/cards/deep/inner.ts",
        "src/other/home.ts",
    ] {
        assert!(dead.contains(&file.to_string()), "{file} in {dead:?}");
    }
}

#[test]
fn globstar_braces_and_negations_are_followed() {
    let report = app(
        "export const routes = import.meta.glob(['./routes/**/*.{ts,tsx}', '!./routes/**/_*.ts']);\n\
         export const eager = import.meta.glob('../lib/*.ts', { eager: true, import: 'default' });\n",
        &[
            ("src/routes/index.tsx", "export {};\n"),
            ("src/routes/blog/post.ts", "export {};\n"),
            // Negated.
            ("src/routes/blog/_draft.ts", "export {};\n"),
            ("lib/util.ts", "export {};\n"),
            // `lib/*.ts` is one level only.
            ("lib/nested/deep.ts", "export {};\n"),
        ],
    );
    let routes = fact(
        &report,
        r#"["./routes/**/*.{ts,tsx}","!./routes/**/_*.ts"]"#,
    );
    assert_eq!(rule(routes), "js.import_meta_glob");
    assert_eq!(
        targets(routes),
        ["src/routes/blog/post.ts", "src/routes/index.tsx"]
    );
    assert_eq!(targets(fact(&report, "../lib/*.ts")), ["lib/util.ts"]);

    let dead = dead(&report);
    assert!(
        dead.contains(&"src/routes/blog/_draft.ts".to_string()),
        "{dead:?}"
    );
    assert!(dead.contains(&"lib/nested/deep.ts".to_string()), "{dead:?}");
    assert!(
        !dead.contains(&"src/routes/blog/post.ts".to_string()),
        "{dead:?}"
    );
    assert!(!dead.contains(&"lib/util.ts".to_string()), "{dead:?}");
}

#[test]
fn root_patterns_start_at_the_vite_root_and_base_moves_relative_ones() {
    let report = app(
        "export const a = import.meta.glob('/src/modules/*.ts');\n\
         export const b = import.meta.glob('./*.ts', { base: './widgets' });\n",
        &[
            ("src/modules/one.ts", "export {};\n"),
            ("src/widgets/w.ts", "export {};\n"),
            // Not under the Vite root's `src/modules`.
            ("other/src/modules/two.ts", "export {};\n"),
        ],
    );
    assert_eq!(
        targets(fact(&report, "/src/modules/*.ts")),
        ["src/modules/one.ts"]
    );
    let based = fact(&report, "./widgets/*.ts");
    assert_eq!(targets(based), ["src/widgets/w.ts"]);
    let dead = dead(&report);
    assert!(
        dead.contains(&"other/src/modules/two.ts".to_string()),
        "{dead:?}"
    );
    assert!(!dead.contains(&"src/widgets/w.ts".to_string()), "{dead:?}");
}

#[test]
fn a_glob_never_matches_its_own_file() {
    let report = app(
        "export const all = import.meta.glob('./*.ts');\n",
        &[("src/sibling.ts", "export {};\n")],
    );
    assert_eq!(targets(fact(&report, "./*.ts")), ["src/sibling.ts"]);
}

#[test]
fn raw_and_url_globs_are_resource_references() {
    let report = app(
        "export const raw = import.meta.glob('./snippets/*.ts', { query: '?raw', import: 'default' });\n",
        &[("src/snippets/a.ts", "export {};\n")],
    );
    let raw = fact(&report, "./snippets/*.ts");
    assert_eq!(raw["kind"], "resource_reference");
    assert_eq!(targets(raw), ["src/snippets/a.ts"]);
    assert!(!dead(&report).contains(&"src/snippets/a.ts".to_string()));
}

#[test]
fn computed_patterns_are_possibilities_with_no_edge() {
    let report = app(
        "const dir = 'screens';\n\
         export const a = import.meta.glob(`./${dir}/*.ts`);\n\
         const pattern = './views/*.ts';\n\
         export const b = import.meta.glob(pattern);\n\
         export const c = import.meta.glob('./parts/*.ts', { base: dir });\n",
        &[
            ("src/screens/home.ts", "export {};\n"),
            ("src/views/v.ts", "export {};\n"),
            ("src/parts/p.ts", "export {};\n"),
        ],
    );
    for (specifier, file) in [
        ("./${dir}/*.ts", "src/screens/home.ts"),
        ("pattern", "src/views/v.ts"),
        ("'./parts/*.ts', { base: dir }", "src/parts/p.ts"),
    ] {
        let f = fact(&report, specifier);
        assert_eq!(f["kind"], "possible_dynamic_load", "{specifier}");
        assert_eq!(f["certainty"], "possibility", "{specifier}");
        assert_eq!(rule(f), "js.import_meta_glob_computed", "{specifier}");
        assert!(dead(&report).contains(&file.to_string()), "{file}");
    }
    assert!(report["dependency_graph"]["./src/main.ts"]
        .as_array()
        .is_none_or(|e| e.is_empty()));
}

#[test]
fn look_alikes_are_not_globs() {
    let report = app(
        "import { glob } from 'tinyglobby';\n\
         const meta = { glob: (p: string) => p };\n\
         // import.meta.glob('./commented/*.ts')\n\
         export const s = \"import.meta.glob('./quoted/*.ts')\";\n\
         export const a = glob('./plain/*.ts');\n\
         export const b = meta.glob('./member/*.ts');\n",
        &[
            ("src/commented/a.ts", "export {};\n"),
            ("src/quoted/a.ts", "export {};\n"),
            ("src/plain/a.ts", "export {};\n"),
            ("src/member/a.ts", "export {};\n"),
        ],
    );
    let dead = dead(&report);
    for file in [
        "src/commented/a.ts",
        "src/quoted/a.ts",
        "src/plain/a.ts",
        "src/member/a.ts",
    ] {
        assert!(dead.contains(&file.to_string()), "{file} in {dead:?}");
    }
    let rules: Vec<&str> = report["dependencies"]["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(rule)
        .collect();
    assert!(
        !rules.iter().any(|r| r.starts_with("js.import_meta_glob")),
        "{rules:?}"
    );
}

#[test]
fn legacy_glob_eager_counts_and_aliases_are_unsupported_possibilities() {
    let report = app(
        "export const a = import.meta.globEager('./legacy/*.ts');\n\
         export const b = import.meta.glob('@/aliased/*.ts');\n",
        &[
            ("src/legacy/l.ts", "export {};\n"),
            ("src/aliased/x.ts", "export {};\n"),
        ],
    );
    assert_eq!(targets(fact(&report, "./legacy/*.ts")), ["src/legacy/l.ts"]);
    let alias = fact(&report, "@/aliased/*.ts");
    assert_eq!(alias["resolution"], "unsupported");
    assert_eq!(alias["certainty"], "possibility");
    assert!(targets(alias).is_empty());
    let dead = dead(&report);
    assert!(!dead.contains(&"src/legacy/l.ts".to_string()), "{dead:?}");
    assert!(dead.contains(&"src/aliased/x.ts".to_string()), "{dead:?}");
}

#[test]
fn a_glob_with_no_match_is_unresolved() {
    let report = app("export const a = import.meta.glob('./none/*.ts');\n", &[]);
    assert_eq!(fact(&report, "./none/*.ts")["resolution"], "unresolved");
}

#[test]
fn glob_targets_follow_new_files_through_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("package.json", r#"{"name":"app","main":"src/main.ts"}"#),
            (MAIN, "export const p = import.meta.glob('./pages/*.ts');\n"),
            ("src/pages/a.ts", "export {};\n"),
        ],
    );
    let first = run(dir.path(), Some(cache.path()));
    assert_eq!(targets(fact(&first, "./pages/*.ts")), ["src/pages/a.ts"]);
    write(dir.path(), &[("src/pages/b.ts", "export {};\n")]);
    let second = run(dir.path(), Some(cache.path()));
    let fresh = run(dir.path(), None);
    assert_eq!(
        targets(fact(&second, "./pages/*.ts")),
        ["src/pages/a.ts", "src/pages/b.ts"]
    );
    assert_eq!(second["dependencies"], fresh["dependencies"]);
}
