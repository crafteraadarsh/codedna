//! Framework adapters through the real CLI: each package's own
//! `package.json` decides which frameworks are active there, and an active
//! adapter names the files its framework loads as `configured_entry` facts
//! in `dependencies.entries`.
//!
//! Every case comes as a pair: the file an adapter makes live, and the same
//! file where the adapter is not active, which stays dead.

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

fn clean(path: &str) -> String {
    path.trim_start_matches("./").to_string()
}

fn dead(report: &Value) -> Vec<String> {
    report["dead_code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| clean(f.as_str().unwrap()))
        .collect()
}

fn entries(report: &Value) -> Vec<Value> {
    report["dependencies"]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The entry fact that names `target`, if any.
fn entry_for(report: &Value, target: &str) -> Option<Value> {
    entries(report).into_iter().find(|f| {
        f["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| clean(t.as_str().unwrap()) == target)
    })
}

fn rule(fact: &Value) -> String {
    fact["evidence"]["rule"].as_str().unwrap().to_string()
}

fn assert_live(report: &Value, file: &str, expected_rule: &str) {
    let dead = dead(report);
    assert!(
        !dead.contains(&file.to_string()),
        "{file} is dead: {dead:?}"
    );
    let fact = entry_for(report, file).unwrap_or_else(|| {
        panic!(
            "no entry fact for {file}: {:#}",
            report["dependencies"]["entries"]
        )
    });
    assert_eq!(rule(&fact), expected_rule, "{fact}");
    assert_eq!(fact["kind"], "configured_entry", "{fact}");
    assert_eq!(fact["resolution"], "resolved", "{fact}");
}

fn assert_dead(report: &Value, file: &str) {
    let dead = dead(report);
    assert!(dead.contains(&file.to_string()), "{file} is live: {dead:?}");
    assert!(
        entry_for(report, file).is_none(),
        "{file} has an entry fact"
    );
}

fn adapters(report: &Value) -> Vec<(String, String)> {
    report["dependencies"]["adapters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|a| {
            (
                clean(a["package"].as_str().unwrap()),
                a["adapter"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

const ROOT: (&str, &str) = (
    "package.json",
    r#"{"name":"root","private":true,"workspaces":["apps/*","packages/*"]}"#,
);
const PAGE: &str = "export default function Page() { return null; }\n";

#[test]
fn next_route_is_live_and_the_same_file_outside_next_is_dead() {
    let report = analyze(&[
        ROOT,
        (
            "apps/web/package.json",
            "{\n  \"name\": \"web\",\n  \"dependencies\": {\n    \"next\": \"15.0.0\",\n    \"react\": \"19.0.0\"\n  }\n}\n",
        ),
        ("apps/web/next.config.js", "module.exports = {};\n"),
        ("apps/web/app/layout.tsx", PAGE),
        ("apps/web/app/dashboard/page.tsx", PAGE),
        ("apps/web/app/(marketing)/about/page.tsx", PAGE),
        ("apps/web/app/api/users/route.ts", "export function GET() {}\n"),
        ("apps/web/app/dashboard/loading.tsx", PAGE),
        ("apps/web/app/_components/unused.tsx", PAGE),
        ("apps/web/middleware.ts", "export function middleware() {}\n"),
        ("apps/web/lib/page.tsx", PAGE),
        (
            "packages/ui/package.json",
            r#"{"name":"ui","dependencies":{"react":"19.0.0"}}"#,
        ),
        ("packages/ui/app/dashboard/page.tsx", PAGE),
        ("packages/ui/app/layout.tsx", PAGE),
        ("packages/ui/middleware.ts", "export function middleware() {}\n"),
    ]);
    assert_live(
        &report,
        "apps/web/app/dashboard/page.tsx",
        "next.app_router@v1",
    );
    assert_live(&report, "apps/web/app/layout.tsx", "next.app_router@v1");
    assert_live(
        &report,
        "apps/web/app/(marketing)/about/page.tsx",
        "next.app_router@v1",
    );
    assert_live(
        &report,
        "apps/web/app/api/users/route.ts",
        "next.app_router@v1",
    );
    assert_live(
        &report,
        "apps/web/app/dashboard/loading.tsx",
        "next.app_router@v1",
    );
    assert_live(&report, "apps/web/middleware.ts", "next.special_file@v1");
    // Private folders and special names outside the router are not routes.
    assert_dead(&report, "apps/web/app/_components/unused.tsx");
    assert_dead(&report, "apps/web/lib/page.tsx");
    // The same files in a package without Next.
    assert_dead(&report, "packages/ui/app/dashboard/page.tsx");
    assert_dead(&report, "packages/ui/app/layout.tsx");

    let fact = entry_for(&report, "apps/web/app/dashboard/page.tsx").unwrap();
    assert_eq!(
        clean(fact["source"].as_str().unwrap()),
        "apps/web/package.json"
    );
    assert_eq!(
        fact["evidence"]["line"], 4,
        "the line declaring next: {fact}"
    );
    assert_eq!(fact["specifier"], "app/dashboard/page.tsx");

    let adapter = report["dependencies"]["adapters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["adapter"] == "next")
        .unwrap()
        .clone();
    assert_eq!(clean(adapter["package"].as_str().unwrap()), "apps/web");
    assert_eq!(adapter["version"], 1);
    assert_eq!(
        clean(adapter["config"][0].as_str().unwrap()),
        "apps/web/next.config.js"
    );
    assert!(!adapters(&report).contains(&("packages/ui".to_string(), "next".to_string())));
}

#[test]
fn an_example_package_with_next_does_not_make_root_routes_live() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"lib","main":"./src/index.ts","devDependencies":{"vitest":"2"}}"#,
        ),
        ("src/index.ts", "export const a = 1;\n"),
        ("app/page.tsx", PAGE),
        ("app/layout.tsx", PAGE),
        (
            "examples/blog/package.json",
            r#"{"name":"blog","dependencies":{"next":"15.0.0"}}"#,
        ),
        ("examples/blog/app/page.tsx", PAGE),
    ]);
    assert_dead(&report, "app/page.tsx");
    assert_dead(&report, "app/layout.tsx");
    assert_live(&report, "examples/blog/app/page.tsx", "next.app_router@v1");
    let next: Vec<_> = adapters(&report)
        .into_iter()
        .filter(|(_, a)| a == "next")
        .collect();
    assert_eq!(
        next,
        vec![("examples/blog".to_string(), "next".to_string())]
    );
}

#[test]
fn next_pages_router_uses_the_root_pages_folder_over_src() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"site","devDependencies":{"next":"14.2.0"}}"#,
        ),
        (
            "next.config.mjs",
            "export default { pageExtensions: ['tsx', 'ts'] };\n",
        ),
        ("pages/index.tsx", PAGE),
        ("pages/_app.tsx", PAGE),
        ("pages/api/hello.ts", "export default function h() {}\n"),
        ("pages/legacy.jsx", PAGE),
        ("src/pages/old.tsx", PAGE),
        ("instrumentation.ts", "export function register() {}\n"),
    ]);
    assert_live(&report, "pages/index.tsx", "next.pages_router@v1");
    assert_live(&report, "pages/_app.tsx", "next.pages_router@v1");
    assert_live(&report, "pages/api/hello.ts", "next.pages_router@v1");
    assert_live(&report, "instrumentation.ts", "next.special_file@v1");
    assert_live(&report, "next.config.mjs", "next.config@v1");
    // `pageExtensions` leaves out `jsx`, and Next ignores `src/pages`
    // when `pages/` exists.
    assert_dead(&report, "pages/legacy.jsx");
    assert_dead(&report, "src/pages/old.tsx");
}

#[test]
fn expo_router_routes_are_live_only_with_expo_router() {
    let report = analyze(&[
        ROOT,
        (
            "apps/mobile/package.json",
            r#"{"name":"mobile","main":"expo-router/entry","dependencies":{"expo":"54","expo-router":"~6.0.0"}}"#,
        ),
        ("apps/mobile/src/app/_layout.tsx", PAGE),
        ("apps/mobile/src/app/(tabs)/index.tsx", PAGE),
        ("apps/mobile/src/app/+not-found.tsx", PAGE),
        (
            "apps/native/package.json",
            r#"{"name":"native","dependencies":{"react-native":"0.81"}}"#,
        ),
        ("apps/native/src/app/(tabs)/home.tsx", PAGE),
        ("apps/native/src/app/_layout.tsx", PAGE),
    ]);
    assert_live(&report, "apps/mobile/src/app/_layout.tsx", "expo.router@v1");
    assert_live(
        &report,
        "apps/mobile/src/app/(tabs)/index.tsx",
        "expo.router@v1",
    );
    // `index.tsx` stays live by filename everywhere; use a plain route name.
    assert_dead(&report, "apps/native/src/app/(tabs)/home.tsx");
    assert_dead(&report, "apps/native/src/app/_layout.tsx");
}

#[test]
fn test_files_are_reported_as_test_used() {
    let report = analyze(&[
        ROOT,
        (
            "packages/a/package.json",
            r#"{"name":"a","main":"./src/index.ts","devDependencies":{"vitest":"2"}}"#,
        ),
        (
            "packages/a/vitest.config.ts",
            "import { defineConfig } from 'vitest/config';\nexport default defineConfig({\n  test: { setupFiles: ['./test/setup.ts'], include: ['test/**/*.ts'] },\n});\n",
        ),
        ("packages/a/src/index.ts", "export const a = 1;\n"),
        ("packages/a/test/setup.ts", "export {};\n"),
        ("packages/a/test/a.ts", "export {};\n"),
        ("packages/a/src/sum.bench.ts", "export {};\n"),
        (
            "packages/b/package.json",
            r#"{"name":"b","main":"./src/index.ts","devDependencies":{"jest":"29"}}"#,
        ),
        ("packages/b/src/index.ts", "export const b = 1;\n"),
        ("packages/b/src/__tests__/b.ts", "export {};\n"),
        (
            "packages/c/package.json",
            r#"{"name":"c","main":"./src/index.ts"}"#,
        ),
        ("packages/c/src/index.ts", "export const c = 1;\n"),
        ("packages/c/src/__tests__/c.ts", "export {};\n"),
        ("packages/c/test/a.ts", "export {};\n"),
        ("packages/c/src/sum.bench.ts", "export {};\n"),
    ]);
    assert_live(&report, "packages/a/test/setup.ts", "test.setup@v1");
    assert_live(&report, "packages/a/test/a.ts", "test.vitest@v1");
    assert_live(&report, "packages/b/src/__tests__/b.ts", "test.jest@v1");
    // `vitest bench` has its own default glob next to `include`.
    assert_live(&report, "packages/a/src/sum.bench.ts", "test.vitest@v1");
    for file in [
        "packages/a/test/setup.ts",
        "packages/a/test/a.ts",
        "packages/b/src/__tests__/b.ts",
    ] {
        assert_eq!(entry_for(&report, file).unwrap()["condition"], "test");
    }
    // Without a test runner nothing loads them.
    assert_dead(&report, "packages/c/src/__tests__/c.ts");
    assert_dead(&report, "packages/c/test/a.ts");
    assert_dead(&report, "packages/c/src/sum.bench.ts");
}

#[test]
fn package_entries_and_scripts_are_entries_only_where_declared() {
    let report = analyze(&[
        ROOT,
        (
            "packages/cli/package.json",
            r#"{"name":"cli","bin":{"cli":"./bin/cli.js"},"exports":{"./plugins/*":"./src/plugins/*.ts"},"scripts":{"seed":"tsx scripts/seed.ts","lint":"eslint src"}}"#,
        ),
        ("packages/cli/bin/cli.js", "console.log(1);\n"),
        ("packages/cli/src/plugins/one.ts", "export {};\n"),
        ("packages/cli/scripts/seed.ts", "export {};\n"),
        ("packages/cli/src/unused.ts", "export {};\n"),
        ("packages/other/package.json", r#"{"name":"other"}"#),
        ("packages/other/bin/cli.js", "console.log(1);\n"),
        ("packages/other/scripts/seed.ts", "export {};\n"),
        (
            "packages/open/package.json",
            r#"{"name":"open","exports":{".":"./src/main.ts","./*":"./*","./feature/*":"./src/feature/*.ts","./feature/secret":null,"./escape":"../outside.ts"}}"#,
        ),
        ("packages/open/src/main.ts", "export {};\n"),
        ("packages/open/src/extra.ts", "export {};\n"),
        ("packages/open/src/feature/a.ts", "export {};\n"),
        ("packages/open/src/feature/secret.ts", "export {};\n"),
        ("packages/outside.ts", "export {};\n"),
    ]);
    assert_live(&report, "packages/cli/bin/cli.js", "node.package_entry@v1");
    assert_live(
        &report,
        "packages/cli/src/plugins/one.ts",
        "node.package_entry@v1",
    );
    assert_live(&report, "packages/cli/scripts/seed.ts", "node.script@v1");
    assert_dead(&report, "packages/cli/src/unused.ts");
    assert_dead(&report, "packages/other/scripts/seed.ts");
    assert_live(
        &report,
        "packages/open/src/main.ts",
        "node.package_entry@v1",
    );
    assert_live(
        &report,
        "packages/open/src/feature/a.ts",
        "node.package_entry@v1",
    );
    // `"./*": "./*"` opens the package but names no entry point; a `null`
    // subpath is blocked; a target outside the package is refused.
    assert_dead(&report, "packages/open/src/extra.ts");
    assert_dead(&report, "packages/open/src/feature/secret.ts");
    assert_dead(&report, "packages/outside.ts");
}
