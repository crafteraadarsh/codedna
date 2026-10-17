//! Package purpose and file roles through the real CLI.
//!
//! Every case comes as a pair: a repository where a folder name or a
//! harness would invent an application layer, and the same files in an
//! application, where the layer stays.

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

fn details(report: &Value) -> &Value {
    &report["architecture_details"]
}

/// `(layer name, file count)` for every layer.
fn layers(report: &Value) -> Vec<(String, u64)> {
    details(report)["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["name"].as_str().unwrap().to_string(),
                l["files"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn layer<'a>(report: &'a Value, name: &str) -> Option<&'a Value> {
    details(report)["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == name)
}

fn files_in(report: &Value, name: &str) -> u64 {
    layer(report, name).map_or(0, |l| l["files"].as_u64().unwrap())
}

/// The purpose of the package rooted at `path` (`.` for the root).
fn purpose(report: &Value, path: &str) -> Option<String> {
    details(report)["packages"]
        .as_array()?
        .iter()
        .find(|p| p["path"] == path)
        .map(|p| p["purpose"].as_str().unwrap().to_string())
}

fn competing(report: &Value) -> Vec<String> {
    details(report)["competing"]
        .as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn evidence(report: &Value) -> String {
    details(report)["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn all_dirs(report: &Value) -> Vec<String> {
    details(report)["layers"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|l| l["dirs"].as_array().unwrap().iter())
        .map(|d| d.as_str().unwrap().to_string())
        .collect()
}

const JS_LAYERS: &[(&str, &str)] = &[
    (
        "lib/middleware/cors.js",
        "module.exports = function cors(req, res, next) { next(); };\n",
    ),
    (
        "lib/middleware/auth.js",
        "module.exports = function auth(req, res, next) { next(); };\n",
    ),
    (
        "lib/routes/index.js",
        "module.exports = { users: require('./users') };\n",
    ),
    (
        "lib/routes/users.js",
        "module.exports = function list() { return []; };\n",
    ),
];

#[test]
fn published_library_folders_are_not_application_layers() {
    let mut files = vec![(
        "package.json",
        r#"{"name":"tiny-mw","version":"1.0.0","main":"lib/index.js","exports":"./lib/index.js","files":["lib"]}"#,
    )];
    files.extend_from_slice(JS_LAYERS);
    let report = analyze(&files);
    assert_eq!(layers(&report), vec![], "{}", details(&report));
    assert_eq!(purpose(&report, ".").as_deref(), Some("library"));
    let competing = competing(&report);
    assert!(
        competing
            .iter()
            .any(|c| c.contains("library") && c.contains("Middleware")),
        "{competing:?}"
    );
}

#[test]
fn application_with_the_same_folders_keeps_its_layers() {
    let mut files = vec![(
        "package.json",
        r#"{"name":"app","private":true,"dependencies":{"express":"^4.18.0"}}"#,
    )];
    files.extend_from_slice(JS_LAYERS);
    let report = analyze(&files);
    assert_eq!(files_in(&report, "Middleware"), 2, "{}", details(&report));
    assert_eq!(files_in(&report, "Routes"), 2);
    assert_eq!(purpose(&report, ".").as_deref(), Some("application"));
    assert!(competing(&report).is_empty());
}

const GO_MIDDLEWARE: &[(&str, &str)] = &[
    ("go.mod", "module example.com/mw\n\ngo 1.21\n"),
    (
        "middleware/logger.go",
        "package middleware\n\nimport \"net/http\"\n\nfunc Logger(next http.Handler) http.Handler {\n\treturn next\n}\n",
    ),
    (
        "middleware/recover.go",
        "package middleware\n\nimport \"net/http\"\n\nfunc Recover(next http.Handler) http.Handler {\n\treturn next\n}\n",
    ),
    (
        "_examples/hello/main.go",
        "package main\n\nimport \"net/http\"\n\nfunc main() {\n\thttp.HandleFunc(\"/hello\", nil)\n}\n",
    ),
    (
        "_examples/rest/routes.go",
        "package main\n\nimport \"net/http\"\n\nfunc routes() {\n\thttp.HandleFunc(\"/articles\", nil)\n}\n",
    ),
];

#[test]
fn go_library_with_examples_has_no_application_layers() {
    let report = analyze(GO_MIDDLEWARE);
    assert_eq!(layers(&report), vec![], "{}", details(&report));
    assert_eq!(purpose(&report, ".").as_deref(), Some("library"));
}

#[test]
fn go_program_keeps_its_middleware_and_skips_examples() {
    let mut files = GO_MIDDLEWARE.to_vec();
    files.push((
        "main.go",
        "package main\n\nimport \"net/http\"\n\nfunc main() {\n\thttp.ListenAndServe(\":8080\", nil)\n}\n",
    ));
    let report = analyze(&files);
    assert_eq!(files_in(&report, "Middleware"), 2, "{}", details(&report));
    assert_eq!(purpose(&report, ".").as_deref(), Some("application"));
    assert!(
        all_dirs(&report).iter().all(|d| !d.contains("_examples")),
        "{:?}",
        all_dirs(&report)
    );
}

const PY_LAYERS: &[(&str, &str)] = &[
    ("pkg/__init__.py", ""),
    ("pkg/views/a.py", "def a():\n    return 1\n"),
    ("pkg/views/b.py", "def b():\n    return 2\n"),
    ("pkg/models/x.py", "X = 1\n"),
    ("pkg/models/y.py", "Y = 2\n"),
];

#[test]
fn python_command_line_tool_is_a_cli_without_layers() {
    let mut files = vec![(
        "pyproject.toml",
        "[project]\nname = \"pkg\"\nversion = \"1.0\"\ndependencies = [\"rich\"]\n\n[project.scripts]\npkg = \"pkg.cli:main\"\n",
    )];
    files.extend_from_slice(PY_LAYERS);
    let report = analyze(&files);
    assert_eq!(purpose(&report, ".").as_deref(), Some("cli"));
    assert_eq!(layers(&report), vec![], "{}", details(&report));
}

#[test]
fn python_web_service_keeps_its_layers() {
    let mut files = vec![(
        "pyproject.toml",
        "[project]\nname = \"pkg\"\nversion = \"1.0\"\ndependencies = [\"fastapi>=0.100\"]\n",
    )];
    files.extend_from_slice(PY_LAYERS);
    let report = analyze(&files);
    assert_eq!(purpose(&report, ".").as_deref(), Some("application"));
    assert_eq!(files_in(&report, "Routes"), 2, "{}", details(&report));
    assert_eq!(files_in(&report, "Data"), 2);
}

#[test]
fn cookiecutter_template_describes_the_generated_project() {
    let report = analyze(&[
        ("cookiecutter.json", r#"{"project_slug":"app"}"#),
        ("hooks/pre_gen_project.py", "print('checking')\n"),
        ("hooks/post_gen_project.py", "print('cleaning')\n"),
        (
            "{{cookiecutter.project_slug}}/app/routes/a.py",
            "def a():\n    return 1\n",
        ),
        (
            "{{cookiecutter.project_slug}}/app/routes/b.py",
            "def b():\n    return 2\n",
        ),
        ("{{cookiecutter.project_slug}}/app/models/x.py", "X = 1\n"),
        ("{{cookiecutter.project_slug}}/app/models/y.py", "Y = 2\n"),
    ]);
    assert_eq!(files_in(&report, "UI"), 0, "{}", details(&report));
    assert_eq!(files_in(&report, "Routes"), 2);
    assert_eq!(files_in(&report, "Data"), 2);
    assert!(
        evidence(&report).contains("template"),
        "{}",
        evidence(&report)
    );
}

#[test]
fn react_hooks_are_still_ui() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"app","private":true,"dependencies":{"react":"^18.0.0"}}"#,
        ),
        (
            "src/hooks/useA.ts",
            "export function useA() { return 1; }\n",
        ),
        (
            "src/hooks/useB.ts",
            "export function useB() { return 2; }\n",
        ),
    ]);
    assert_eq!(files_in(&report, "UI"), 2, "{}", details(&report));
    assert!(!evidence(&report).contains("template"));
}

#[test]
fn cli_templates_are_not_the_cli_architecture() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["cli","www"]}"#,
        ),
        (
            "cli/package.json",
            r#"{"name":"create-thing","version":"1.0.0","bin":{"create-thing":"index.js"},"files":["index.js","template"],"dependencies":{"commander":"^11.0.0"}}"#,
        ),
        ("cli/index.js", "console.log('creating');\n"),
        (
            "cli/template/src/pages/index.tsx",
            "export default function P() { return <div />; }\n",
        ),
        (
            "cli/template/src/components/A.tsx",
            "export function A() { return <div />; }\n",
        ),
        (
            "cli/template/src/components/B.tsx",
            "export function B() { return <div />; }\n",
        ),
        (
            "www/package.json",
            r#"{"name":"www","private":true,"dependencies":{"astro":"^4.0.0"}}"#,
        ),
        ("www/src/components/X.astro", "<div>x</div>\n"),
        ("www/src/components/Y.astro", "<div>y</div>\n"),
    ]);
    assert_eq!(purpose(&report, "cli").as_deref(), Some("cli"));
    assert_eq!(purpose(&report, "www").as_deref(), Some("application"));
    assert_eq!(files_in(&report, "UI"), 2, "{}", details(&report));
    assert!(all_dirs(&report).iter().all(|d| !d.contains("template")));
}

#[test]
fn cli_templates_count_when_nothing_else_has_layers() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"create-thing","version":"1.0.0","bin":{"create-thing":"index.js"},"files":["index.js","template"],"dependencies":{"commander":"^11.0.0"}}"#,
        ),
        ("index.js", "console.log('creating');\n"),
        (
            "template/src/components/A.tsx",
            "export function A() { return <div />; }\n",
        ),
        (
            "template/src/components/B.tsx",
            "export function B() { return <div />; }\n",
        ),
    ]);
    assert_eq!(purpose(&report, ".").as_deref(), Some("cli"));
    assert_eq!(files_in(&report, "UI"), 2, "{}", details(&report));
    assert!(
        evidence(&report).contains("template"),
        "{}",
        evidence(&report)
    );
}

const MOCK_HANDLERS: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{"name":"app","private":true,"dependencies":{"react":"^18.0.0","express":"^4.18.0"}}"#,
    ),
    (
        "src/testing/mocks/handlers/users.ts",
        "export const users = [];\n",
    ),
    (
        "src/testing/mocks/handlers/auth.ts",
        "export const auth = [];\n",
    ),
    ("src/testing/mocks/db.ts", "export const db = {};\n"),
    (
        "src/app/Home.stories.tsx",
        "export const Home = () => <div />;\n",
    ),
    (
        "src/app/Card.stories.tsx",
        "export const Card = () => <div />;\n",
    ),
];

#[test]
fn mock_handlers_and_stories_are_not_layers() {
    let report = analyze(MOCK_HANDLERS);
    assert_eq!(layers(&report), vec![], "{}", details(&report));
}

#[test]
fn real_routes_beside_mocks_stay() {
    let mut files = MOCK_HANDLERS.to_vec();
    files.push((
        "server/routes/users.js",
        "const router = require('express').Router();\nrouter.get('/users', (req, res) => res.json([]));\nmodule.exports = router;\n",
    ));
    files.push((
        "server/routes/auth.js",
        "const router = require('express').Router();\nrouter.post('/login', (req, res) => res.json({}));\nmodule.exports = router;\n",
    ));
    let report = analyze(&files);
    assert_eq!(files_in(&report, "Routes"), 2, "{}", details(&report));
    assert_eq!(files_in(&report, "Data"), 0);
    assert!(all_dirs(&report).iter().all(|d| !d.contains("testing")));
}

fn elixir(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|i| {
            (
                format!("lib/app/web/controllers/page{i}_controller.ex"),
                format!("defmodule App.Page{i}Controller do\n  use AppWeb, :controller\nend\n"),
            )
        })
        .collect()
}

fn components(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|i| {
            (
                format!("assets/js/components/C{i}.tsx"),
                format!("export function C{i}() {{ return <div />; }}\n"),
            )
        })
        .collect()
}

fn owned(files: &[(String, String)]) -> Vec<(&str, &str)> {
    files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect()
}

#[test]
fn architecture_is_unknown_when_unread_languages_dominate() {
    let mut files = elixir(25);
    files.extend(components(3));
    files.push((
        "mix.exs".into(),
        "defmodule App.MixProject do\nend\n".into(),
    ));
    let report = analyze(&owned(&files));
    assert_eq!(layers(&report), vec![], "{}", details(&report));
    assert_eq!(details(&report)["confidence"], "low");
    let unknown = details(&report)["unknown"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(unknown.contains("Elixir"), "{}", details(&report));
    assert!(
        competing(&report).iter().any(|c| c.contains("UI")),
        "{:?}",
        competing(&report)
    );
}

#[test]
fn a_little_unread_code_does_not_hide_the_layers() {
    let mut files = components(25);
    files.extend(elixir(2));
    files.push((
        "package.json".into(),
        r#"{"name":"app","private":true,"dependencies":{"react":"^18.0.0"}}"#.into(),
    ));
    let report = analyze(&owned(&files));
    assert_eq!(files_in(&report, "UI"), 25, "{}", details(&report));
    assert!(details(&report).get("unknown").is_none());
}

#[test]
fn a_file_with_routes_and_a_model_shows_both_roles() {
    let report = analyze(&[
        ("package.json", r#"{"name":"app","private":true,"dependencies":{"express":"^4.18.0","mongoose":"^8.0.0"}}"#),
        (
            "src/routes/users.js",
            "const mongoose = require('mongoose');\nconst router = require('express').Router();\nconst User = mongoose.model('User', new mongoose.Schema({ name: String }));\nrouter.get('/users', async (req, res) => res.json(await User.find()));\nmodule.exports = router;\n",
        ),
        (
            "src/routes/items.js",
            "const router = require('express').Router();\nrouter.get('/items', (req, res) => res.json([]));\nmodule.exports = router;\n",
        ),
    ]);
    let routes = layer(&report, "Routes").expect("routes layer");
    let example = |file: &str| {
        routes["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["file"].as_str().unwrap().ends_with(file))
            .cloned()
            .unwrap_or_else(|| panic!("{file} in {routes}"))
    };
    let users = example("users.js");
    let also: Vec<&str> = users["also"]
        .as_array()
        .map(|a| a.iter().map(|r| r["layer"].as_str().unwrap()).collect())
        .unwrap_or_default();
    assert_eq!(also, vec!["Data"], "{users}");
    assert_eq!(users["also"][0]["code"]["line"], 3);
    assert!(example("items.js").get("also").is_none());
    assert_eq!(routes["also_shown"]["Data"], 1, "{routes}");
}

#[test]
fn django_packages_hooks_and_migrations_take_their_real_roles() {
    let report = analyze(&[
        ("manage.py", "import django\n"),
        ("app/__init__.py", "from app.models import x\n"),
        ("app/apps.py", "from app.models import x\n"),
        (
            "app/views/a.py",
            "import app\nimport app.apps\nfrom app import billing\n",
        ),
        ("app/views/b.py", "from app import billing\n"),
        ("app/billing.py", "from app.models import x\n"),
        ("app/models/x.py", "X = 1\n"),
        ("app/models/y.py", "Y = 2\n"),
        ("app/migrations/0004_alter_options_domain.py", "OPS = []\n"),
        ("app/wagtail_hooks.py", "HOOKS = []\n"),
        ("blog/wagtail_hooks.py", "HOOKS = []\n"),
        (
            "src/hooks/useA.ts",
            "export function useA() { return 1; }\n",
        ),
        (
            "src/hooks/useB.ts",
            "export function useB() { return 2; }\n",
        ),
    ]);
    assert_eq!(files_in(&report, "Services"), 1, "{}", details(&report));
    assert_eq!(files_in(&report, "Data"), 3);
    assert_eq!(files_in(&report, "UI"), 2);
}

/// A workspace with an express app and two published packages: shared
/// models it uses, and a client SDK for outside callers.
fn product_monorepo(app_manifest: &str) -> Vec<(&str, String)> {
    let model = |name: &str| {
        format!("const mongoose = require('mongoose');\nmodule.exports = mongoose.model('{name}', new mongoose.Schema({{}}));\n")
    };
    vec![
        ("package.json", r#"{"name":"root","private":true,"workspaces":["apps/*","packages/*"]}"#.to_string()),
        ("apps/api/package.json", app_manifest.to_string()),
        ("apps/api/routes/users.js", "const router = require('express').Router();\nrouter.get('/users', (req, res) => res.json([]));\nmodule.exports = router;\n".to_string()),
        ("packages/schemas/package.json", r#"{"name":"@acme/schemas","version":"1.0.0","exports":"./index.js","files":["models"]}"#.to_string()),
        ("packages/schemas/models/user.js", model("User")),
        ("packages/schemas/models/team.js", model("Team")),
        ("packages/sdk/package.json", r#"{"name":"@acme/sdk","version":"1.0.0","exports":"./index.js","files":["models"]}"#.to_string()),
        ("packages/sdk/models/user.js", model("User")),
    ]
}

fn borrowed<'a>(files: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    files.iter().map(|(p, c)| (*p, c.as_str())).collect()
}

#[test]
fn shared_packages_of_an_application_count_but_its_sdk_does_not() {
    let files =
        product_monorepo(r#"{"name":"api","private":true,"dependencies":{"express":"^4.18.0"}}"#);
    let report = analyze(&borrowed(&files));
    assert_eq!(purpose(&report, "apps/api").as_deref(), Some("application"));
    assert_eq!(
        purpose(&report, "packages/schemas").as_deref(),
        Some("library")
    );
    assert_eq!(files_in(&report, "Data"), 2, "{}", details(&report));
    assert!(
        all_dirs(&report)
            .iter()
            .all(|d| !d.contains("packages/sdk")),
        "{:?}",
        all_dirs(&report)
    );
    let competing = competing(&report);
    assert!(
        competing
            .iter()
            .any(|c| c.starts_with("packages/sdk is a library")),
        "{competing:?}"
    );
}

#[test]
fn without_an_application_the_same_packages_are_libraries_only() {
    let files = product_monorepo(
        r#"{"name":"api","version":"1.0.0","exports":"./index.js","files":["routes"]}"#,
    );
    let report = analyze(&borrowed(&files));
    assert_eq!(purpose(&report, "apps/api").as_deref(), Some("library"));
    assert_eq!(layers(&report), vec![], "{}", details(&report));
}

#[test]
fn an_app_nested_in_a_library_repository_is_a_demo() {
    let report = analyze(&[
        (
            "package.json",
            r#"{"name":"forms","version":"1.0.0","exports":"./src/index.ts","files":["dist"]}"#,
        ),
        ("src/index.ts", "export function useForm() { return {}; }\n"),
        (
            "app/package.json",
            r#"{"name":"app","private":true,"dependencies":{"express":"^4.18.0"}}"#,
        ),
        (
            "app/routes/a.js",
            "const router = require('express').Router();\nrouter.get('/a', (req, res) => res.end());\nmodule.exports = router;\n",
        ),
        (
            "app/routes/b.js",
            "const router = require('express').Router();\nrouter.get('/b', (req, res) => res.end());\nmodule.exports = router;\n",
        ),
    ]);
    assert_eq!(purpose(&report, ".").as_deref(), Some("library"));
    assert_eq!(purpose(&report, "app").as_deref(), Some("application"));
    assert_eq!(layers(&report), vec![], "{}", details(&report));
}

#[test]
fn bundled_browser_app_with_index_html_is_an_application() {
    let manifest = r#"{"name":"@acme/app","version":"1.0.0","exports":{".":"./dist/index.html"},"files":["dist"],"scripts":{"dev":"vite --host","build":"vite build"},"devDependencies":{"vue":"^3.4.0","vite":"^5.0.0"}}"#;
    let mut files = vec![
        ("app/package.json", manifest),
        ("app/index.html", "<div id=\"app\"></div>\n"),
    ];
    let vue = "<template><div>x</div></template>\n";
    files.extend_from_slice(&[
        ("app/src/components/a.vue", vue),
        ("app/src/components/b.vue", vue),
        (
            "package.json",
            r#"{"name":"root","private":true,"workspaces":["app"]}"#,
        ),
    ]);
    let report = analyze(&files);
    assert_eq!(
        purpose(&report, "app").as_deref(),
        Some("application"),
        "{}",
        details(&report)
    );
    assert_eq!(files_in(&report, "UI"), 2);

    // Without index.html the same manifest is a published library.
    files.retain(|(p, _)| *p != "app/index.html");
    let report = analyze(&files);
    assert_eq!(purpose(&report, "app").as_deref(), Some("library"));
    assert_eq!(layers(&report), vec![], "{}", details(&report));
}

#[test]
fn a_library_wsgi_adapter_is_not_a_server_entry() {
    let pyproject = "[project]\nname = \"client\"\nversion = \"1.0\"\ndependencies = [\"idna\"]\n";
    let mut files = vec![
        ("pyproject.toml", pyproject),
        ("client/__init__.py", ""),
        (
            "client/transports/wsgi.py",
            "class WSGITransport:\n    def handle(self, request):\n        return request\n",
        ),
    ];
    files.extend_from_slice(PY_LAYERS);
    let report = analyze(&files);
    assert_eq!(
        purpose(&report, ".").as_deref(),
        Some("library"),
        "{}",
        details(&report)
    );
    assert_eq!(layers(&report), vec![]);

    // A wsgi.py that defines the application makes it a server.
    files.push((
        "client/wsgi.py",
        "from client import create\n\napplication = create()\n",
    ));
    let report = analyze(&files);
    assert_eq!(purpose(&report, ".").as_deref(), Some("application"));
    assert_eq!(files_in(&report, "Routes"), 2, "{}", details(&report));
}
