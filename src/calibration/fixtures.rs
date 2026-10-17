//! Fully annotated fixture repositories (`calibration/fixtures.txt`).
//!
//! Every file in a fixture is known to be dead or in use, so dead-code
//! recall is exact here. Each fixture is written to a temporary folder and
//! analyzed three times: with the cache off, with a cold cache and with a
//! warm one. All three reports must agree.

use super::{normal, protocol};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

const SOURCE: &str = include_str!("../../calibration/fixtures.txt");

/// One fixture as written in `fixtures.txt`.
#[derive(Debug, Default)]
pub struct Fixture {
    pub name: String,
    pub about: String,
    /// Files nothing uses.
    pub dead: Vec<String>,
    /// `(check, file)` the audit must report.
    pub audit: Vec<(String, String)>,
    pub files: Vec<(String, String)>,
}

/// Parse the shipped fixtures.
pub fn parse() -> Vec<Fixture> {
    parse_text(SOURCE)
}

fn parse_text(text: &str) -> Vec<Fixture> {
    let mut fixtures: Vec<Fixture> = Vec::new();
    let mut lines: Option<Vec<&str>> = None;
    let mut path = String::new();
    let flush = |fixtures: &mut Vec<Fixture>, path: &str, lines: Option<Vec<&str>>| {
        if let (Some(mut lines), Some(f)) = (lines, fixtures.last_mut()) {
            while lines.last() == Some(&"") {
                lines.pop();
            }
            f.files
                .push((path.to_string(), format!("{}\n", lines.join("\n"))));
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("@@@ ") {
            flush(&mut fixtures, &path, lines.take());
            let (key, value) = rest.split_once(' ').unwrap_or((rest, ""));
            let value = value.trim().to_string();
            match key {
                "fixture" => fixtures.push(Fixture {
                    name: value,
                    ..Fixture::default()
                }),
                _ => {
                    let Some(f) = fixtures.last_mut() else {
                        continue;
                    };
                    match key {
                        "about" => f.about = value,
                        "dead" => f.dead.push(value),
                        "audit" => {
                            if let Some((check, file)) = value.split_once(' ') {
                                f.audit.push((check.to_string(), file.trim().to_string()));
                            }
                        }
                        "file" => {
                            path = value;
                            lines = Some(Vec::new());
                        }
                        _ => {}
                    }
                }
            }
        } else if let Some(lines) = lines.as_mut() {
            lines.push(line);
        }
    }
    flush(&mut fixtures, &path, lines.take());
    fixtures
}

#[derive(Debug, Serialize)]
pub struct FalsePositive {
    pub file: String,
    pub confidence: String,
}

#[derive(Debug, Serialize)]
pub struct Control {
    pub check: String,
    pub file: String,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FixtureResult {
    pub name: String,
    pub about: String,
    pub files: usize,
    pub dead: usize,
    pub found: usize,
    pub missed: Vec<String>,
    pub false_positives: Vec<FalsePositive>,
    pub controls: Vec<Control>,
    pub cache_agrees: bool,
}

#[derive(Debug, Serialize)]
pub struct DeadTotals {
    pub dead: usize,
    pub found: usize,
    pub false_positives: usize,
    pub recall: f64,
    pub precision: f64,
}

#[derive(Debug, Serialize)]
pub struct ControlTotals {
    pub total: usize,
    pub found: usize,
}

#[derive(Debug, Serialize)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub fixtures: Vec<FixtureResult>,
    pub dead_code: DeadTotals,
    pub controls: ControlTotals,
    pub gates: Vec<GateResult>,
    pub passed: bool,
}

fn write(root: &Path, fixture: &Fixture) -> std::io::Result<()> {
    // Older than the cache's racy window, so the cache stores every file.
    let old = SystemTime::now() - Duration::from_secs(3600);
    for (path, text) in &fixture.files {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, text)?;
        std::fs::File::options()
            .write(true)
            .open(&target)?
            .set_modified(old)?;
    }
    Ok(())
}

/// Analysis and audit as one comparable value.
fn snapshot(root: &Path) -> serde_json::Value {
    let result = crate::analysis::analyze(root);
    let audit = crate::audit::scan(root, None);
    serde_json::json!({
        "analysis": serde_json::to_value(&result).unwrap_or_default(),
        "audit": serde_json::to_value(&audit.findings).unwrap_or_default(),
    })
}

/// Run every fixture. Uses the process-wide cache switch and
/// `CODEDNA_CACHE_DIR`, so call it only from the CLI.
pub fn run() -> std::io::Result<Report> {
    let work = tempfile::tempdir()?;
    let cache = work.path().join("cache");
    std::fs::create_dir_all(&cache)?;
    std::env::set_var("CODEDNA_CACHE_DIR", &cache);
    std::env::remove_var("CODEDNA_NO_CACHE");

    let mut results = Vec::new();
    for fixture in parse() {
        let root = work.path().join(&fixture.name);
        write(&root, &fixture)?;

        crate::cache::disable();
        let off = snapshot(&root);
        crate::cache::enable();
        let cold = snapshot(&root);
        let warm = snapshot(&root);
        crate::cache::disable();
        let cache_agrees = off == cold && cold == warm;

        let result = crate::analysis::analyze(&root);
        let reported: Vec<(String, String)> = result
            .dead_code_details
            .iter()
            .map(|c| {
                (
                    normal(&c.file.to_string_lossy()),
                    c.confidence.as_str().to_string(),
                )
            })
            .collect();
        let dead: BTreeSet<String> = fixture.dead.iter().map(|d| normal(d)).collect();
        let found = reported.iter().filter(|(f, _)| dead.contains(f)).count();
        let reported_files: BTreeSet<&str> = reported.iter().map(|(f, _)| f.as_str()).collect();
        let missed = dead
            .iter()
            .filter(|d| !reported_files.contains(d.as_str()))
            .cloned()
            .collect();
        let false_positives = reported
            .iter()
            .filter(|(f, _)| !dead.contains(f))
            .map(|(file, confidence)| FalsePositive {
                file: file.clone(),
                confidence: confidence.clone(),
            })
            .collect();

        let findings = crate::audit::scan(&root, None).findings;
        let controls = fixture
            .audit
            .iter()
            .map(|(check, file)| {
                let hit = findings
                    .iter()
                    .find(|f| f.check == check && normal(&f.file) == normal(file));
                Control {
                    check: check.clone(),
                    file: file.clone(),
                    found: hit.is_some(),
                    confidence: hit.map(|f| f.confidence.as_str().to_string()),
                }
            })
            .collect();

        results.push(FixtureResult {
            name: fixture.name.clone(),
            about: fixture.about.clone(),
            files: fixture.files.len(),
            dead: dead.len(),
            found,
            missed,
            false_positives,
            controls,
            cache_agrees,
        });
    }
    // Agreement means nothing if the cache never stored anything.
    let stored = walkdir_count(&cache);
    Ok(summarize(results, stored))
}

fn walkdir_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| {
                    let path = e.path();
                    if path.is_dir() {
                        walkdir_count(&path)
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

fn summarize(fixtures: Vec<FixtureResult>, stored: usize) -> Report {
    let dead: usize = fixtures.iter().map(|f| f.dead).sum();
    let found: usize = fixtures.iter().map(|f| f.found).sum();
    let false_positives: usize = fixtures.iter().map(|f| f.false_positives.len()).sum();
    let recall = if dead == 0 {
        0.0
    } else {
        found as f64 / dead as f64
    };
    let precision = if found + false_positives == 0 {
        0.0
    } else {
        found as f64 / (found + false_positives) as f64
    };
    let total = fixtures.iter().map(|f| f.controls.len()).sum();
    let controls_found = fixtures
        .iter()
        .flat_map(|f| &f.controls)
        .filter(|c| c.found)
        .count();
    let min_recall = protocol().fixtures.min_dead_code_recall;
    let disagree: Vec<&str> = fixtures
        .iter()
        .filter(|f| !f.cache_agrees)
        .map(|f| f.name.as_str())
        .collect();
    let gates = vec![
        GateResult {
            name: "Dead-code recall".to_string(),
            passed: dead > 0 && recall >= min_recall,
            detail: format!(
                "{found} of {dead} dead files found ({:.0}%, needs {:.0}%)",
                recall * 100.0,
                min_recall * 100.0
            ),
        },
        GateResult {
            name: "Positive controls".to_string(),
            passed: controls_found == total,
            detail: format!("{controls_found} of {total} audit controls reported"),
        },
        GateResult {
            name: "Cache on and off agree".to_string(),
            passed: disagree.is_empty() && stored > 0,
            detail: if stored == 0 {
                "the cache stored nothing, so nothing was compared".to_string()
            } else if disagree.is_empty() {
                format!(
                    "{} fixtures identical uncached, cold and warm",
                    fixtures.len()
                )
            } else {
                format!("differs in {}", disagree.join(", "))
            },
        },
    ];
    let passed = gates.iter().all(|g| g.passed);
    Report {
        dead_code: DeadTotals {
            dead,
            found,
            false_positives,
            recall,
            precision,
        },
        controls: ControlTotals {
            total,
            found: controls_found,
        },
        fixtures,
        gates,
        passed,
    }
}

/// Plain-text summary.
pub fn render(report: &Report) -> String {
    let mut out = String::from("Fixture evaluation (fully annotated repositories)\n\n");
    for f in &report.fixtures {
        out.push_str(&format!(
            "  {:<20} {:>2}/{:<2} dead found, {} false positive{}{}\n",
            f.name,
            f.found,
            f.dead,
            f.false_positives.len(),
            if f.false_positives.len() == 1 {
                ""
            } else {
                "s"
            },
            if f.cache_agrees {
                ""
            } else {
                ", cache disagrees"
            }
        ));
        for m in &f.missed {
            out.push_str(&format!("  {:<20} missed {m}\n", ""));
        }
        for fp in &f.false_positives {
            out.push_str(&format!(
                "  {:<20} wrongly reported {} ({})\n",
                "", fp.file, fp.confidence
            ));
        }
        for c in f.controls.iter().filter(|c| !c.found) {
            out.push_str(&format!(
                "  {:<20} control not reported: {} in {}\n",
                "", c.check, c.file
            ));
        }
    }
    let d = &report.dead_code;
    out.push_str(&format!(
        "\n  Dead code: recall {:.0}%, precision {:.0}% ({} dead, {} found, {} false positives)\n\n",
        d.recall * 100.0,
        d.precision * 100.0,
        d.dead,
        d.found,
        d.false_positives
    ));
    for g in &report.gates {
        out.push_str(&format!(
            "  {} {}: {}\n",
            if g.passed { "pass" } else { "FAIL" },
            g.name,
            g.detail
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture_blocks() {
        let text = "# comment\n@@@ fixture a\n@@@ about An app\n@@@ dead x.js\n\
                    @@@ audit secret .env\n@@@ file x.js\nlet a = 1;\n\n\n@@@ file .env\nK=v\n";
        let f = parse_text(text);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].about, "An app");
        assert_eq!(f[0].dead, ["x.js"]);
        assert_eq!(f[0].audit, [("secret".to_string(), ".env".to_string())]);
        assert_eq!(
            f[0].files[0],
            ("x.js".to_string(), "let a = 1;\n".to_string())
        );
        assert_eq!(f[0].files[1].1, "K=v\n");
    }

    #[test]
    fn shipped_fixtures_name_existing_files() {
        let fixtures = parse();
        assert!(fixtures.len() >= 7);
        for f in &fixtures {
            let files: Vec<&str> = f.files.iter().map(|(p, _)| p.as_str()).collect();
            for d in &f.dead {
                assert!(files.contains(&d.as_str()), "{}: {d}", f.name);
            }
            for (_, a) in &f.audit {
                assert!(files.contains(&a.as_str()), "{}: {a}", f.name);
            }
        }
    }
}
