//! JavaScript and TypeScript dependencies come from a real parser: only
//! import and export declarations, `require` and `import()` calls and the
//! runtime loaders in code count, never text that looks like them in a
//! string, a comment or component markup. Every case comes as a pair: the
//! form that must be found, and the look-alike that must not.

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

fn facts_for<'a>(report: &'a Value, specifier: &str) -> Vec<&'a Value> {
    facts(report)
        .iter()
        .filter(|f| f["specifier"] == specifier)
        .collect()
}

/// The one fact for `specifier`.
fn fact<'a>(report: &'a Value, specifier: &str) -> &'a Value {
    let found = facts_for(report, specifier);
    assert_eq!(
        found.len(),
        1,
        "one fact for {specifier} in {:#}",
        report["dependencies"]
    );
    found[0]
}

fn edges<'a>(report: &'a Value, source: &str) -> Vec<&'a str> {
    report["dependency_graph"][source]
        .as_array()
        .unwrap_or_else(|| panic!("{source} is not a node: {:#}", report["dependency_graph"]))
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect()
}

fn rule(f: &Value) -> &str {
    f["evidence"]["rule"].as_str().unwrap()
}

#[test]
fn reports_the_new_rules_version() {
    let report = analyze(&[("a.ts", "export {};\n")]);
    assert_eq!(report["dependencies"]["rules_version"], 2);
}

#[test]
fn multiline_renamed_namespace_and_type_only_imports() {
    let index = "import {\n\
                 \x20 a as alpha,\n\
                 \x20 b,\n\
                 } from \"./a\";\n\
                 import * as ns from \"./ns\";\n\
                 import def, { c } from \"./def\";\n\
                 export * from \"./star\";\n\
                 export * as nsx from \"./nsx\";\n\
                 export {\n\
                 \x20 x,\n\
                 } from \"./x\";\n\
                 import type { T } from \"./types\";\n\
                 export type { U } from \"./utypes\";\n\
                 import { type V } from \"./inline\";\n\
                 const lazy = () => import(\n\
                 \x20 \"./lazy\"\n\
                 );\n\
                 type M = typeof import(\"./typed\");\n\
                 export type * from \"./alltypes\";\n";
    let mut files = vec![("src/index.ts", index)];
    for name in [
        "a", "ns", "def", "star", "nsx", "x", "types", "utypes", "inline", "lazy", "typed",
        "alltypes",
    ] {
        files.push((
            Box::leak(format!("src/{name}.ts").into_boxed_str()),
            "export {};\n",
        ));
    }
    let report = analyze(&files);

    let a = fact(&report, "./a");
    assert_eq!(a["kind"], "runtime_import");
    assert_eq!(a["certainty"], "direct");
    assert_eq!(a["evidence"]["line"], 4);
    assert_eq!(rule(a), "js.import");
    assert_eq!(rule(fact(&report, "./ns")), "js.import");
    assert_eq!(rule(fact(&report, "./def")), "js.import");
    for re_export in ["./star", "./nsx", "./x"] {
        assert_eq!(fact(&report, re_export)["kind"], "re_export", "{re_export}");
        assert_eq!(rule(fact(&report, re_export)), "js.export_from");
    }
    assert_eq!(fact(&report, "./x")["evidence"]["line"], 11);
    assert_eq!(fact(&report, "./types")["kind"], "type_only_import");
    assert_eq!(rule(fact(&report, "./types")), "js.import_type");
    assert_eq!(fact(&report, "./utypes")["kind"], "type_only_import");
    assert_eq!(rule(fact(&report, "./utypes")), "js.export_type_from");
    assert_eq!(fact(&report, "./alltypes")["kind"], "type_only_import");
    // Only some names typed: the import itself still loads the module.
    assert_eq!(fact(&report, "./inline")["kind"], "runtime_import");
    let lazy = fact(&report, "./lazy");
    assert_eq!(lazy["condition"], "dynamic");
    assert_eq!(lazy["evidence"]["line"], 16);
    let typed = fact(&report, "./typed");
    assert_eq!(typed["kind"], "type_only_import");
    assert_eq!(rule(typed), "js.import_type_expression");

    assert_eq!(edges(&report, "./src/index.ts").len(), 12);
}

#[test]
fn strings_comments_and_regexes_are_not_imports() {
    let look_alikes = "const s = \"import x from './ghost1'\";\n\
                       const t = `require(\"./ghost2\")`;\n\
                       // import \"./ghost3\";\n\
                       /* export * from \"./ghost4\" */\n\
                       const msg = 'loaded from \"./ghost5\"';\n\
                       const re = /import(\"\\.\\/ghost6\")/;\n\
                       /**\n\
                       \x20* @example import { y } from \"./ghost7\";\n\
                       \x20*/\n\
                       import { real } from \"./real\";\n";
    let mut files = vec![
        ("src/index.js", look_alikes),
        ("src/real.js", "export const real = 1;\n"),
    ];
    for n in 1..=7 {
        files.push((
            Box::leak(format!("src/ghost{n}.js").into_boxed_str()),
            "export {};\n",
        ));
    }
    let report = analyze(&files);
    for n in 1..=7 {
        assert!(
            facts_for(&report, &format!("./ghost{n}")).is_empty(),
            "ghost{n} in {:#}",
            report["dependencies"]
        );
    }
    assert_eq!(edges(&report, "./src/index.js"), ["./src/real.js"]);
    assert_eq!(fact(&report, "./real")["evidence"]["line"], 10);
}

#[test]
fn shadowed_require_is_only_a_possibility() {
    let report = analyze(&[
        (
            "src/amd.js",
            "function load(require) {\n  return require(\"./shadowed\");\n}\n\
             const real = require(\"./real\");\n",
        ),
        (
            "src/made.mjs",
            "import { createRequire } from \"node:module\";\n\
             const require = createRequire(import.meta.url);\n\
             const made = require(\"./made-target.cjs\");\n",
        ),
        (
            "src/ambient.ts",
            "declare const require: (id: string) => unknown;\n\
             const x = require(\"./ambient-target\");\n",
        ),
        ("src/shadowed.js", "module.exports = 1;\n"),
        ("src/real.js", "module.exports = 1;\n"),
        ("src/made-target.cjs", "module.exports = 1;\n"),
        ("src/ambient-target.ts", "export {};\n"),
    ]);
    let shadowed = fact(&report, "./shadowed");
    assert_eq!(shadowed["certainty"], "possibility");
    assert_eq!(rule(shadowed), "js.require_shadowed");
    assert_eq!(edges(&report, "./src/amd.js"), ["./src/real.js"]);
    assert_eq!(rule(fact(&report, "./real")), "js.require");

    assert_eq!(fact(&report, "./made-target.cjs")["certainty"], "direct");
    assert_eq!(edges(&report, "./src/made.mjs"), ["./src/made-target.cjs"]);
    assert_eq!(fact(&report, "./ambient-target")["certainty"], "direct");
    assert_eq!(
        edges(&report, "./src/ambient.ts"),
        ["./src/ambient-target.ts"]
    );
}

#[test]
fn shadowed_worker_is_only_a_possibility() {
    let report = analyze(&[
        (
            "src/pool.ts",
            "import { Worker } from \"./my-pool\";\nnew Worker(\"./job.js\");\n",
        ),
        (
            "src/threads.ts",
            "import { Worker as NodeWorker } from \"node:worker_threads\";\n\
             new NodeWorker(\"./thread.js\");\n\
             import * as wt from \"worker_threads\";\n\
             new wt.Worker(\"./other.js\");\n",
        ),
        (
            "src/my-pool.ts",
            "export class Worker { constructor(_: string) {} }\n",
        ),
        ("src/job.js", "export {};\n"),
        ("src/thread.js", "export {};\n"),
        ("src/other.js", "export {};\n"),
    ]);
    let job = fact(&report, "./job.js");
    assert_eq!(job["certainty"], "possibility");
    assert_eq!(rule(job), "js.worker_shadowed");
    assert_eq!(edges(&report, "./src/pool.ts"), ["./src/my-pool.ts"]);

    let thread = fact(&report, "./thread.js");
    assert_eq!(thread["kind"], "worker_entry");
    assert_eq!(thread["certainty"], "inferred");
    assert_eq!(fact(&report, "./other.js")["kind"], "worker_entry");
    assert_eq!(
        edges(&report, "./src/threads.ts"),
        ["./src/thread.js", "./src/other.js"]
    );
}

#[test]
fn renamed_and_required_process_loaders_count() {
    let report = analyze(&[
        (
            "src/run.ts",
            "import { spawn as run } from \"node:child_process\";\n\
             run(\"tsx\", [\"./job.ts\"]);\n\
             const cp = require(\"child_process\");\n\
             cp.fork(\"./forked.js\");\n\
             const { execSync } = require(\"node:child_process\");\n\
             execSync(\"node ./cmd.mjs --flag\");\n",
        ),
        (
            "src/local.ts",
            "const run = (cmd: string, args: string[]) => [cmd, args];\n\
             run(\"tsx\", [\"./not-a-job.ts\"]);\n",
        ),
        ("src/job.ts", "export {};\n"),
        ("src/forked.js", "export {};\n"),
        ("src/cmd.mjs", "export {};\n"),
        ("src/not-a-job.ts", "export {};\n"),
    ]);
    for loaded in ["./job.ts", "./forked.js", "./cmd.mjs"] {
        assert_eq!(fact(&report, loaded)["kind"], "process_entry", "{loaded}");
    }
    assert_eq!(fact(&report, "./cmd.mjs")["evidence"]["line"], 6);
    assert!(facts_for(&report, "./not-a-job.ts").is_empty());
    assert!(edges(&report, "./src/local.ts").is_empty());
}

#[test]
fn cjs_mts_and_cts_files_are_parsed_and_resolved() {
    let report = analyze(&[
        ("src/a.cjs", "const b = require(\"./b.cjs\");\n"),
        ("src/b.cjs", "module.exports = 1;\n"),
        ("src/m.mts", "import { n } from \"./n.mjs\";\n"),
        ("src/n.mts", "export const n = 1;\n"),
        ("src/c.cts", "import d = require(\"./d.cjs\");\n"),
        ("src/d.cts", "export = 1;\n"),
    ]);
    assert_eq!(edges(&report, "./src/a.cjs"), ["./src/b.cjs"]);
    assert_eq!(edges(&report, "./src/m.mts"), ["./src/n.mts"]);
    assert_eq!(edges(&report, "./src/c.cts"), ["./src/d.cts"]);
    assert_eq!(rule(fact(&report, "./d.cjs")), "js.import_equals");
    assert_eq!(report["coverage"]["source_files"], 6);
    assert_eq!(report["coverage"]["analyzed_files"], 6);
    assert_eq!(report["coverage"]["parse"]["parsed"], 6);
}

#[test]
fn component_scripts_are_parsed_and_markup_is_not() {
    let vue = "<template>\n\
               \x20 <p>import x from \"./ghost-vue\"</p>\n\
               </template>\n\
               <!-- <script>import \"./commented\";</script> -->\n\
               <script setup lang=\"ts\">\n\
               import Child from \"./Child.vue\";\n\
               const n: number = 1;\n\
               </script>\n";
    let svelte = "<script context=\"module\">\n\
                  \x20 export { helper } from \"./helper.js\";\n\
                  </script>\n\
                  <p>import y from \"./ghost-svelte\"</p>\n";
    let astro = "---\n\
                 import Layout from \"./Layout.astro\";\n\
                 const title: string = \"x\";\n\
                 ---\n\
                 <Layout>import z from \"./ghost-astro\"</Layout>\n\
                 <script>\n\
                 \x20 import \"./client.ts\";\n\
                 </script>\n";
    let report = analyze(&[
        ("src/App.vue", vue),
        ("src/Child.vue", "<template><p/></template>\n"),
        ("src/commented.js", "export {};\n"),
        ("src/ghost-vue.js", "export {};\n"),
        ("src/Widget.svelte", svelte),
        ("src/helper.js", "export const helper = 1;\n"),
        ("src/ghost-svelte.js", "export {};\n"),
        ("src/Page.astro", astro),
        ("src/Layout.astro", "<slot />\n"),
        ("src/client.ts", "export {};\n"),
        ("src/ghost-astro.js", "export {};\n"),
    ]);
    for ghost in [
        "./ghost-vue",
        "./commented",
        "./ghost-svelte",
        "./ghost-astro",
    ] {
        assert!(facts_for(&report, ghost).is_empty(), "{ghost}");
    }
    let child = fact(&report, "./Child.vue");
    assert_eq!(child["evidence"]["line"], 6);
    assert_eq!(edges(&report, "./src/App.vue"), ["./src/Child.vue"]);
    assert_eq!(fact(&report, "./helper.js")["kind"], "re_export");
    assert_eq!(fact(&report, "./helper.js")["evidence"]["line"], 2);
    assert_eq!(edges(&report, "./src/Widget.svelte"), ["./src/helper.js"]);
    assert_eq!(fact(&report, "./client.ts")["evidence"]["line"], 7);
    assert_eq!(
        edges(&report, "./src/Page.astro"),
        ["./src/Layout.astro", "./src/client.ts"]
    );
}

#[test]
fn component_spans_point_into_the_original_file() {
    let dir = tempfile::tempdir().unwrap();
    let vue = "<template><div/></template>\n<script>\nimport a from \"./a.js\";\n</script>\n";
    write(
        dir.path(),
        &[("App.vue", vue), ("a.js", "export default 1;\n")],
    );
    let report = run(dir.path(), None);
    let span = &fact(&report, "./a.js")["evidence"]["span"];
    let (start, end) = (
        span[0].as_u64().unwrap() as usize,
        span[1].as_u64().unwrap() as usize,
    );
    assert_eq!(&vue[start..end], "./a.js");
}

#[test]
fn interpolated_and_computed_specifiers_are_possibilities() {
    let report = analyze(&[
        (
            "src/index.js",
            "const page = (name) => import(`./pages/${name}.js`);\n\
             const fixed = import(`./fixed`);\n\
             const joined = import(\"./parts\" + \"/joined\");\n\
             const any = (name) => import(name);\n\
             const old = (name) => require(\"./legacy/\" + name);\n",
        ),
        ("src/pages/home.js", "export {};\n"),
        ("src/fixed.js", "export {};\n"),
        ("src/parts/joined.js", "export {};\n"),
    ]);
    let page = fact(&report, "./pages/${name}.js");
    assert_eq!(page["certainty"], "possibility");
    assert_eq!(page["resolution"], "unsupported");
    assert_eq!(rule(page), "js.template_import");

    let fixed = fact(&report, "./fixed");
    assert_eq!(fixed["certainty"], "direct");
    assert_eq!(fixed["condition"], "dynamic");
    // Concatenated literals are a constant, folded into one path.
    let joined = fact(&report, "./parts/joined");
    assert_eq!(joined["certainty"], "direct");

    let any = fact(&report, "name");
    assert_eq!(any["certainty"], "possibility");
    assert_eq!(rule(any), "js.computed_import");
    let old = fact(&report, "\"./legacy/\" + name");
    assert_eq!(old["certainty"], "possibility");
    assert_eq!(rule(old), "js.computed_require");

    assert_eq!(
        edges(&report, "./src/index.js"),
        ["./src/fixed.js", "./src/parts/joined.js"]
    );
}

#[test]
fn files_with_recoverable_syntax_errors_are_partial() {
    let report = analyze(&[
        ("src/ok.ts", "import { a } from \"./a\";\n"),
        (
            "src/broken.ts",
            "import { a } from \"./a\";\nconst x = a ?? 1 || 2;\n",
        ),
        ("src/a.ts", "export const a = 1;\n"),
    ]);
    let parse = &report["coverage"]["parse"];
    assert_eq!(parse["parsed"], 2, "{parse:#}");
    assert_eq!(parse["partial"], 1, "{parse:#}");
    assert_eq!(parse["failed"], 0, "{parse:#}");
    let failure = &parse["failures"][0];
    assert_eq!(failure["status"], "partial");
    assert_eq!(failure["file"], "src/broken.ts");
    assert_eq!(failure["line"], 2);
    assert!(!failure["message"].as_str().unwrap().is_empty());
    assert!(report["coverage"]["summary"]
        .as_str()
        .unwrap()
        .contains("did not parse"));
    assert_eq!(edges(&report, "./src/ok.ts"), ["./src/a.ts"]);
    // The recovered tree still holds the import.
    assert_eq!(edges(&report, "./src/broken.ts"), ["./src/a.ts"]);
}

#[test]
fn a_fatal_parse_error_falls_back_to_possibilities_only() {
    let report = analyze(&[
        (
            "src/broken.js",
            "import { a } from \"./a.js\";\nconst x = require(\"./b.js\");\n<<<<<<< HEAD\n",
        ),
        ("src/a.js", "export const a = 1;\n"),
        ("src/b.js", "module.exports = 1;\n"),
    ]);
    assert_eq!(report["coverage"]["parse"]["failed"], 1);
    for specifier in ["./a.js", "./b.js"] {
        let f = fact(&report, specifier);
        assert_eq!(f["certainty"], "possibility", "{f}");
        assert_eq!(rule(f), "js.text_scan");
    }
    assert!(edges(&report, "./src/broken.js").is_empty());
}

/// Facts with what a reformat must not change: everything but positions.
fn shape(report: &Value) -> Vec<String> {
    let mut shapes: Vec<String> = facts(report)
        .iter()
        .map(|f| {
            format!(
                "{} {} {} {} {} {} {}",
                f["source"],
                f["specifier"],
                f["kind"],
                f["resolution"],
                f["certainty"],
                f["evidence"]["rule"],
                f["targets"]
            )
        })
        .collect();
    shapes.sort();
    shapes
}

#[test]
fn comments_whitespace_and_renames_do_not_change_facts() {
    let base = "import { a } from \"./a\";\n\
                import * as b from \"./b\";\n\
                export { c } from \"./c\";\n\
                const d = require(\"./d\");\n\
                const e = () => import(\"./e\");\n";
    let edited = "// header comment mentioning import \"./ghost\"\n\n\
                  import {\n  a as renamedA, // trailing\n} from './a'\n\
                  /* block */ import * as other from \"./b\"\n\n\n\
                  export {\n  c,\n} from \"./c\";\n\
                  const   d   =   require( './d' )\n\
                  const e = () =>\n  import(\n    \"./e\"\n  )\n";
    let support = [
        ("src/a.ts", "export const a = 1;\n"),
        ("src/b.ts", "export const b = 1;\n"),
        ("src/c.ts", "export const c = 1;\n"),
        ("src/d.ts", "export const d = 1;\n"),
        ("src/e.ts", "export const e = 1;\n"),
    ];
    let mut one = vec![("src/index.ts", base)];
    one.extend(support);
    let mut two = vec![("src/index.ts", edited)];
    two.extend(support);
    let (a, b) = (analyze(&one), analyze(&two));
    assert_eq!(shape(&a), shape(&b));
    assert_eq!(a["dependency_graph"], b["dependency_graph"]);
}

#[test]
fn parse_results_match_with_the_cache_on_and_off() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "src/index.ts",
                "import { a } from \"./a\";\nimport(`./p/${x}`);\n",
            ),
            ("src/a.ts", "export const a = 1;\n"),
            (
                "src/broken.ts",
                "import { a } from \"./a\";\nconst x = a ?? 1 || 2;\n",
            ),
            ("src/fatal.js", "require(\"./a\");\n<<<<<<< HEAD\n"),
            (
                "src/App.vue",
                "<script setup>\nimport \"./a\";\n</script>\n",
            ),
        ],
    );
    let off = run(dir.path(), None);
    let cold = run(dir.path(), Some(cache.path()));
    let warm = run(dir.path(), Some(cache.path()));
    for report in [&cold, &warm] {
        assert_eq!(off["dependencies"], report["dependencies"]);
        assert_eq!(off["dependency_graph"], report["dependency_graph"]);
        assert_eq!(off["coverage"], report["coverage"]);
    }
    assert_eq!(off["coverage"]["parse"]["failed"], 1);
    assert_eq!(
        off["coverage"]["parse"]["failures"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn possibility_facts_add_no_edges() {
    let report = analyze(&[
        (
            "src/index.js",
            "function f(require) { return require(\"./a.js\"); }\n",
        ),
        ("src/a.js", "export {};\n"),
    ]);
    let f = fact(&report, "./a.js");
    assert_eq!(f["certainty"], "possibility");
    assert_eq!(f["resolution"], "resolved");
    assert_eq!(f["targets"], json!(["./src/a.js"]));
    assert!(edges(&report, "./src/index.js").is_empty());
}

#[test]
fn jsdoc_type_imports_count_in_javascript_only() {
    let js = "/** @import { Builder } from './builder' */\n\
              /** @typedef {import(\"./types.d.ts\").Config} Config */\n\
              // @type {import('./ghost1').T}\n\
              /* @import { G } from './ghost2' */\n\
              const s = \"/** @type {import('./ghost3').T} */\";\n\
              export const a = 1;\n";
    let ts = "/** @type {import('./ghost4').T} */\nexport const b = 1;\n";
    let report = analyze(&[
        ("src/index.js", js),
        ("src/other.ts", ts),
        ("src/builder.js", "export {};\n"),
        ("src/types.d.ts", "export type Config = {};\n"),
        ("src/ghost1.js", "export {};\n"),
        ("src/ghost2.js", "export {};\n"),
        ("src/ghost3.js", "export {};\n"),
        ("src/ghost4.ts", "export {};\n"),
    ]);
    let builder = fact(&report, "./builder");
    assert_eq!(builder["kind"], "type_only_import");
    assert_eq!(builder["certainty"], "direct");
    assert_eq!(builder["evidence"]["line"], 1);
    assert_eq!(rule(builder), "js.jsdoc_import");
    let types = fact(&report, "./types.d.ts");
    assert_eq!(types["targets"], json!(["./src/types.d.ts"]));
    assert_eq!(types["evidence"]["line"], 2);
    for n in 1..=4 {
        assert!(
            facts_for(&report, &format!("./ghost{n}")).is_empty(),
            "ghost{n} in {:#}",
            report["dependencies"]
        );
    }
    assert_eq!(
        edges(&report, "./src/index.js"),
        ["./src/builder.js", "./src/types.d.ts"]
    );
}

#[test]
fn jsdoc_type_imports_resolve_declarations_and_skip_stale_or_self_refs() {
    let js = "/** @typedef {import('./types').Config} Config */\n\
              /** @typedef {import('./internal.js').Inner} Inner */\n\
              /** @typedef {import('./index.js').Self} Self */\n\
              /** @typedef {import('./$types').PageLoad} PageLoad */\n\
              /** @import { Kit } from '@sveltejs/kit' */\n\
              export const a = 1;\n";
    let report = analyze(&[
        ("src/index.js", js),
        ("src/types.d.ts", "export type Config = {};\n"),
        ("src/internal.d.ts", "export type Inner = {};\n"),
    ]);
    assert_eq!(
        fact(&report, "./types")["targets"],
        json!(["./src/types.d.ts"])
    );
    assert_eq!(
        fact(&report, "./internal.js")["targets"],
        json!(["./src/internal.d.ts"])
    );
    assert!(facts_for(&report, "./index.js").is_empty());
    assert!(facts_for(&report, "./$types").is_empty());
    assert_eq!(fact(&report, "@sveltejs/kit")["resolution"], "external");
    assert_eq!(
        edges(&report, "./src/index.js"),
        ["./src/types.d.ts", "./src/internal.d.ts"]
    );
    assert_eq!(report["coverage"]["imports"]["unresolved"], 0);
}
