//! Measured confidence: how often each confidence level was right.
//!
//! Each set under `calibration/` pairs pinned repositories (`corpus.json`)
//! with hand-checked findings from them (`labels.json`). Each label says
//! whether a dead-code candidate, audit finding or architecture result was
//! correct. Precision per level comes with a 95% Wilson interval, so a level
//! checked on few findings shows a wide range.
//!
//! A set stays independent only until a detector is changed after reading its
//! labels. The `fresh` set was labelled after the last such change; the
//! `original` set was used to fix detectors in 0.2.7.
//!
//! `codedna calibration` prints the numbers recorded for each set.
//! `--corpus DIR` re-runs this build on the repositories cloned in `DIR` and
//! scores the labels against what it reports now.

use crate::architecture::Confidence;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One labelled set of repositories.
pub struct Set {
    pub name: &'static str,
    /// What the set is and whether it is still held out.
    pub about: &'static str,
    corpus: &'static str,
    labels: &'static str,
}

/// Every set, the one still held out first.
pub const SETS: [Set; 2] = [
    Set {
        name: "fresh",
        about: "labelled with 0.2.8; no detector tuned on it (0.2.9 only stops listing missing Django apps.py files)",
        corpus: include_str!("../calibration/fresh/corpus.json"),
        labels: include_str!("../calibration/fresh/labels.json"),
    },
    Set {
        name: "original",
        about: "labelled with 0.2.6; 0.2.7 fixed detectors after reading these labels",
        corpus: include_str!("../calibration/corpus.json"),
        labels: include_str!("../calibration/labels.json"),
    },
];

/// Kinds of result that carry a confidence, in display order.
const KINDS: [&str; 3] = ["dead_code", "audit", "architecture"];

/// Levels in display order; `not reported` is a label the current build no
/// longer reports.
const LEVELS: [&str; 4] = ["high", "medium", "low", "not reported"];

/// One pinned repository.
#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub repo: String,
    pub url: String,
    pub commit: String,
}

/// One hand-checked result.
#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    /// `dead_code`, `audit` or `architecture`.
    pub kind: String,
    pub repo: String,
    /// The file it is about; empty for architecture.
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: Option<usize>,
    /// The audit check, e.g. `SECRET`.
    #[serde(default)]
    pub check: Option<String>,
    /// The level CodeDna gave it when it was labelled.
    pub confidence: String,
    /// `correct`, `incorrect` or `unsure`.
    pub verdict: String,
}

/// Precision of one level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelStat {
    pub level: String,
    pub correct: usize,
    /// Labelled `correct` or `incorrect`; `unsure` is left out.
    pub checked: usize,
    pub precision: f64,
    /// 95% Wilson interval.
    pub low: f64,
    pub high: f64,
}

/// Precision per level for one kind of result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KindStat {
    pub kind: String,
    pub levels: Vec<LevelStat>,
    pub unsure: usize,
}

/// The whole measurement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calibration {
    /// Which labelled set: `fresh` or `original`.
    pub set: String,
    /// What the set is and whether it is still held out.
    pub about: String,
    /// `published` (the labels as recorded) or `measured` (this build, re-run).
    pub source: String,
    pub repositories: usize,
    pub labels: usize,
    pub kinds: Vec<KindStat>,
    /// Repositories missing from `--corpus DIR`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Set {
    /// The pinned repositories.
    pub fn corpus(&self) -> Vec<Repo> {
        serde_json::from_str(self.corpus).expect("calibration corpus is valid")
    }

    /// The labels shipped with this build.
    pub fn labels(&self) -> Vec<Label> {
        serde_json::from_str(self.labels).expect("calibration labels are valid")
    }
}

/// The sets named `name`, or every set.
pub fn sets(name: Option<&str>) -> Vec<&'static Set> {
    SETS.iter()
        .filter(|s| name.is_none_or(|n| n == s.name))
        .collect()
}

/// Precision per level, using the level each label was recorded at.
pub fn published(set: &Set) -> Calibration {
    let labels = set.labels();
    let levels: Vec<String> = labels.iter().map(|l| l.confidence.clone()).collect();
    score(
        set,
        "published",
        set.corpus().len(),
        &labels,
        &levels,
        Vec::new(),
    )
}

/// Re-run this build on the repositories cloned under `dir` (one folder per
/// repository, named as in the corpus) and score the labels against the
/// levels it reports now.
pub fn measure(set: &Set, dir: &Path) -> Calibration {
    let labels = set.labels();
    let repos = set.corpus();
    let mut missing = Vec::new();
    let mut now: HashMap<&str, Current> = HashMap::new();
    for repo in &repos {
        let root = dir.join(&repo.repo);
        if !root.is_dir() {
            missing.push(repo.repo.clone());
            continue;
        }
        eprintln!("Measuring {}…", repo.repo);
        now.insert(&repo.repo, Current::of(&root));
    }
    let levels: Vec<String> = labels
        .iter()
        .map(|l| match now.get(l.repo.as_str()) {
            Some(current) => current.level(l).unwrap_or("not reported").to_string(),
            None => String::new(),
        })
        .collect();
    let (labels, levels): (Vec<Label>, Vec<String>) = labels
        .into_iter()
        .zip(levels)
        .filter(|(_, level)| !level.is_empty())
        .unzip();
    score(
        set,
        "measured",
        repos.len() - missing.len(),
        &labels,
        &levels,
        missing,
    )
}

/// What this build reports for one repository.
struct Current {
    dead: HashMap<String, Confidence>,
    audit: Vec<crate::audit::Finding>,
    architecture: Confidence,
}

impl Current {
    fn of(root: &Path) -> Self {
        let result = crate::analysis::analyze(root);
        let dead = result
            .dead_code_details
            .iter()
            .map(|c| (normal(&c.file.to_string_lossy()), c.confidence))
            .collect();
        Current {
            dead,
            audit: crate::audit::scan(root, None).findings,
            architecture: result.architecture_details.confidence,
        }
    }

    /// The level this build gives the labelled result, or `None` when it no
    /// longer reports it.
    fn level(&self, label: &Label) -> Option<&'static str> {
        let file = normal(&label.file);
        let found = match label.kind.as_str() {
            "dead_code" => self.dead.get(&file).copied(),
            "audit" => self
                .audit
                .iter()
                .find(|f| {
                    normal(&f.file) == file
                        && label.check.as_deref() == Some(f.check)
                        && (label.line.is_none() || f.line == label.line)
                })
                .map(|f| f.confidence),
            "architecture" => Some(self.architecture),
            _ => None,
        };
        found.map(Confidence::as_str)
    }
}

/// `./src/a.ts` → `src/a.ts`, with forward slashes.
fn normal(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.strip_prefix("./").unwrap_or(&path).to_string()
}

/// Count verdicts per kind and level.
fn score(
    set: &Set,
    source: &str,
    repositories: usize,
    labels: &[Label],
    levels: &[String],
    missing: Vec<String>,
) -> Calibration {
    let kinds = KINDS
        .iter()
        .map(|&kind| {
            let mut unsure = 0;
            let levels = LEVELS
                .iter()
                .filter_map(|&level| {
                    let (mut correct, mut checked) = (0, 0);
                    for (label, at) in labels.iter().zip(levels) {
                        if label.kind != kind || at != level {
                            continue;
                        }
                        match label.verdict.as_str() {
                            "correct" => {
                                correct += 1;
                                checked += 1;
                            }
                            "incorrect" => checked += 1,
                            _ => unsure += 1,
                        }
                    }
                    (checked > 0).then(|| {
                        let (low, high) = wilson(correct, checked);
                        LevelStat {
                            level: level.to_string(),
                            correct,
                            checked,
                            precision: correct as f64 / checked as f64,
                            low,
                            high,
                        }
                    })
                })
                .collect();
            KindStat {
                kind: kind.to_string(),
                levels,
                unsure,
            }
        })
        .collect();
    Calibration {
        set: set.name.to_string(),
        about: set.about.to_string(),
        source: source.to_string(),
        repositories,
        labels: labels.len(),
        kinds,
        missing,
    }
}

/// 95% Wilson score interval for `correct` out of `checked`.
pub fn wilson(correct: usize, checked: usize) -> (f64, f64) {
    if checked == 0 {
        return (0.0, 1.0);
    }
    let z = 1.959_964_f64;
    let n = checked as f64;
    let p = correct as f64 / n;
    let centre = p + z * z / (2.0 * n);
    let margin = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    let denom = 1.0 + z * z / n;
    (
        ((centre - margin) / denom).max(0.0),
        ((centre + margin) / denom).min(1.0),
    )
}

/// Plain-text table.
pub fn render(c: &Calibration) -> String {
    let mut out = format!(
        "Measured confidence, {} set ({} labels from {} repositories{})\n  {}\n\n",
        c.set,
        c.labels,
        c.repositories,
        if c.source == "measured" {
            ", re-run with this build"
        } else {
            ""
        },
        c.about
    );
    for kind in &c.kinds {
        let name = match kind.kind.as_str() {
            "dead_code" => "Dead code",
            "audit" => "Audit",
            _ => "Architecture",
        };
        for (i, s) in kind.levels.iter().enumerate() {
            out.push_str(&format!(
                "  {:<13} {:<13} {:>3}/{:<3} {:>4.0}%  ({:.0}–{:.0}%)\n",
                if i == 0 { name } else { "" },
                s.level,
                s.correct,
                s.checked,
                s.precision * 100.0,
                s.low * 100.0,
                s.high * 100.0
            ));
        }
        if kind.unsure > 0 {
            out.push_str(&format!("  {:<13} {} unsure, left out\n", "", kind.unsure));
        }
    }
    out.push_str(
        "\nPrecision: of the findings at that level, how many were right. \
         The range is a 95% interval; few findings give a wide one.\n",
    );
    if c.kinds
        .iter()
        .any(|k| k.levels.iter().any(|s| s.level == "not reported"))
    {
        out.push_str(
            "\"not reported\": labelled findings this build no longer reports; \
             a low number there means it dropped false alarms.\n",
        );
    }
    if !c.missing.is_empty() {
        out.push_str("\nNot found in the corpus folder; clone each at its pinned commit:\n");
        let corpus = sets(Some(&c.set))
            .iter()
            .flat_map(|s| s.corpus())
            .collect::<Vec<_>>();
        for repo in corpus.iter().filter(|r| c.missing.contains(&r.repo)) {
            out.push_str(&format!(
                "  git clone {} {} && git -C {} checkout {}\n",
                repo.url, repo.repo, repo.repo, repo.commit
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_matches_known_values() {
        let (lo, hi) = wilson(8, 10);
        assert!((lo - 0.490).abs() < 0.001, "{lo}");
        assert!((hi - 0.943).abs() < 0.001, "{hi}");
        assert_eq!(wilson(0, 0), (0.0, 1.0));
        let (lo, hi) = wilson(5, 5);
        assert!(lo > 0.5 && (hi - 1.0).abs() < 1e-9);
    }

    #[test]
    fn shipped_labels_parse_and_point_at_the_corpus() {
        let mut all_repos = std::collections::HashSet::new();
        for set in &SETS {
            let repos: Vec<String> = set.corpus().into_iter().map(|r| r.repo).collect();
            let labels = set.labels();
            assert!(!labels.is_empty());
            for label in &labels {
                assert!(repos.contains(&label.repo), "{}", label.repo);
                assert!(KINDS.contains(&label.kind.as_str()));
                assert!(["correct", "incorrect", "unsure"].contains(&label.verdict.as_str()));
            }
            // A held-out set shares no repository with another set.
            for repo in repos {
                assert!(all_repos.insert(repo.clone()), "{repo} is in two sets");
            }
        }
        assert_eq!(sets(Some("fresh")).len(), 1);
        assert_eq!(sets(None).len(), SETS.len());
    }

    #[test]
    fn scores_per_kind_and_level() {
        let label = |kind: &str, verdict: &str| Label {
            kind: kind.to_string(),
            repo: "r".to_string(),
            file: String::new(),
            line: None,
            check: None,
            confidence: "high".to_string(),
            verdict: verdict.to_string(),
        };
        let labels = vec![
            label("dead_code", "correct"),
            label("dead_code", "incorrect"),
            label("dead_code", "unsure"),
            label("audit", "correct"),
        ];
        let levels: Vec<String> = ["high", "high", "high", "not reported"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = score(&SETS[0], "published", 1, &labels, &levels, Vec::new());
        let dead = &c.kinds[0];
        assert_eq!(dead.levels.len(), 1);
        assert_eq!((dead.levels[0].correct, dead.levels[0].checked), (1, 2));
        assert_eq!(dead.unsure, 1);
        assert_eq!(c.kinds[1].levels[0].level, "not reported");
        assert!(c.kinds[2].levels.is_empty());
        assert!(render(&c).contains("Dead code"));
    }

    #[test]
    fn paths_compare_without_leading_dot() {
        assert_eq!(normal("./src/a.ts"), "src/a.ts");
        assert_eq!(normal("src\\a.ts"), "src/a.ts");
    }
}
