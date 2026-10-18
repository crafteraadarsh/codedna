//! The evaluation process behind `codedna calibration`:
//!
//! - the fully annotated fixtures give exact dead-code recall and precision,
//!   check the audit's positive controls and check that a cached run agrees
//!   with an uncached one; `--check` fails when a gate fails;
//! - the sealed set cannot be re-measured by accident: `--corpus` on it needs
//!   `--release`, which shows aggregates only, and a labelling queue is
//!   refused for it;
//! - a baseline written from the development sets checks cleanly against
//!   the same build;
//! - published numbers carry each set's role and the release gate status of
//!   every level;
//! - reports say whether each kind's `high` level passed the release gate.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn codedna(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(args)
        .output()
        .unwrap()
}

fn json_of(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn fixtures_pass_their_gates() {
    let cache = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_codedna"))
        .args(["calibration", "--fixtures", "--check", "--json"])
        .env("CODEDNA_CACHE_DIR", cache.path())
        .output()
        .unwrap();
    let report = json_of(&output);
    let fixtures = report["fixtures"].as_array().unwrap();
    assert!(fixtures.len() >= 7, "{fixtures:?}");
    for fixture in fixtures {
        assert_eq!(fixture["cache_agrees"], true, "{fixture}");
    }
    let dead = &report["dead_code"];
    assert!(dead["dead"].as_u64().unwrap() >= 15, "{dead}");
    assert!(dead["recall"].as_f64().unwrap() >= 0.8, "{dead}");
    assert_eq!(
        report["controls"]["found"], report["controls"]["total"],
        "{report}"
    );
    let gates = report["gates"].as_array().unwrap();
    assert!(gates.len() >= 3);
    assert!(gates.iter().all(|g| g["passed"] == true), "{gates:?}");
    assert_eq!(report["passed"], true);
}

#[test]
fn fixtures_text_names_each_gate() {
    let output = codedna(&["calibration", "--fixtures"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Dead-code recall"), "{text}");
    assert!(text.contains("Positive controls"), "{text}");
    assert!(text.contains("Cache on and off agree"), "{text}");
}

#[test]
fn sealed_set_needs_release_to_be_measured() {
    let corpus = tempfile::tempdir().unwrap();
    let dir = corpus.path().to_str().unwrap();
    let output = codedna(&["calibration", "--set", "third", "--corpus", dir]);
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("sealed") && err.contains("--release"), "{err}");

    // With --release it measures, aggregates only.
    let output = codedna(&[
        "calibration",
        "--set",
        "third",
        "--corpus",
        dir,
        "--release",
        "--json",
    ]);
    let all = json_of(&output);
    let third = &all[0];
    assert_eq!(third["role"], "sealed");
    assert!(third.get("stale").is_none(), "{third}");
    assert!(third.get("new_findings").is_none(), "{third}");
}

#[test]
fn whole_corpus_run_skips_the_sealed_set() {
    let corpus = tempfile::tempdir().unwrap();
    let output = codedna(&[
        "calibration",
        "--corpus",
        corpus.path().to_str().unwrap(),
        "--json",
    ]);
    let all = json_of(&output);
    let names: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["set"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"third"), "{names:?}");
    assert!(names.contains(&"fresh"));
}

#[test]
fn queue_is_refused_for_the_sealed_set() {
    let corpus = tempfile::tempdir().unwrap();
    let queue = corpus.path().join("queue.json");
    let output = codedna(&[
        "calibration",
        "--set",
        "third",
        "--corpus",
        corpus.path().to_str().unwrap(),
        "--release",
        "--queue",
        queue.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(!queue.exists());
}

#[test]
fn written_baseline_checks_cleanly() {
    let corpus = tempfile::tempdir().unwrap();
    let dir = corpus.path().to_str().unwrap();
    let baseline = corpus.path().join("baseline.json");
    let file = baseline.to_str().unwrap();
    let output = codedna(&[
        "calibration",
        "--role",
        "development",
        "--corpus",
        dir,
        "--write-baseline",
        file,
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Value = serde_json::from_slice(&std::fs::read(&baseline).unwrap()).unwrap();
    assert_eq!(written["sets"], serde_json::json!(["fresh", "original"]));

    let output = codedna(&[
        "calibration",
        "--role",
        "development",
        "--corpus",
        dir,
        "--baseline",
        file,
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("No regressions"), "{text}");
}

#[test]
fn shipped_development_baseline_names_the_development_sets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let baseline: Value = serde_json::from_slice(
        &std::fs::read(root.join("calibration/baseline/development.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(baseline["sets"], serde_json::json!(["fresh", "original"]));
    assert!(baseline["labels"].as_array().unwrap().len() > 500);
    assert!(baseline["repositories"].as_object().unwrap().len() >= 80);
}

#[test]
fn published_numbers_carry_role_and_gate() {
    let all = json_of(&codedna(&["calibration", "--json"]));
    for set in all.as_array().unwrap() {
        let role = set["role"].as_str().unwrap();
        assert!(["development", "calibration", "sealed"].contains(&role));
        for kind in set["kinds"].as_array().unwrap() {
            for level in kind["levels"].as_array().unwrap() {
                let gate = level["gate"].as_str().unwrap();
                assert!(
                    ["validated", "unvalidated", "insufficient_data", "not_gated"].contains(&gate),
                    "{gate}"
                );
                if level["level"] != "high" {
                    assert_eq!(gate, "not_gated");
                }
            }
            assert!(kind["selective"].is_array(), "{kind}");
        }
    }
    let text = String::from_utf8(codedna(&["calibration"]).stdout).unwrap();
    assert!(text.contains("sealed"), "{text}");
    assert!(text.contains("insufficient data"), "{text}");
}

#[test]
fn reports_say_whether_high_confidence_is_validated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.py"), "print('hi')\n").unwrap();
    let report = json_of(&codedna(&[
        "json",
        "--compact",
        "--no-cache",
        dir.path().to_str().unwrap(),
    ]));
    let checks = report["analyzer"]["confidence"].as_array().unwrap();
    let kinds: Vec<&str> = checks.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["dead_code", "audit", "architecture"]);
    for check in checks {
        assert_eq!(check["level"], "high");
        assert_eq!(check["set"], "third");
        assert!(check["outcomes"].is_u64());
        assert!(["validated", "unvalidated", "insufficient_data"]
            .contains(&check["status"].as_str().unwrap()));
    }
}
