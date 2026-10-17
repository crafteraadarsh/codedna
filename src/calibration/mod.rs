//! Measured confidence: how often each confidence level was right.
//!
//! Each set under `calibration/` pairs pinned repositories (`corpus.json`)
//! with hand-checked results from them (`labels.json`). Each label says
//! whether a dead-code candidate, audit finding or architecture result was
//! correct; a `no_finding` label says whether a file CodeDna did not report
//! should have been reported, which is what recall is estimated from.
//!
//! `calibration/protocol.json` gives every set a role:
//!
//! - **development**: labels may be read and detectors changed after
//!   reading them;
//! - **calibration**: only used to set confidence thresholds;
//! - **sealed**: the release test set, measured once per release with
//!   `--release`, aggregates only.
//!
//! It also records each set's exposures (who read it, when, and whether
//! details were used for tuning) and the gate a level must pass before it
//! is called validated. A sealed or calibration set becomes a development
//! set as soon as its failures are inspected.
//!
//! Precision per level comes with a 95% Wilson interval and a repository
//! bootstrap interval, so a level checked on few findings, or on findings
//! from few repositories, shows a wide range.
//!
//! `codedna calibration` prints the numbers recorded for each set.
//! `--corpus DIR` re-runs this build on the repositories cloned in `DIR` and
//! scores the labels against what it reports now. `--fixtures` runs the
//! fully annotated fixtures in `calibration/fixtures.txt`.

pub mod baseline;
pub mod fixtures;
pub mod stats;

use crate::architecture::Confidence;
use serde::{Deserialize, Serialize};
use stats::{Evidence, Gate, GateStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

/// One labelled set of repositories.
pub struct Set {
    pub name: &'static str,
    corpus: &'static str,
    labels: &'static str,
}

/// Every set, the sealed one first.
pub const SETS: [Set; 3] = [
    Set {
        name: "third",
        corpus: include_str!("../../calibration/third/corpus.json"),
        labels: include_str!("../../calibration/third/labels.json"),
    },
    Set {
        name: "fresh",
        corpus: include_str!("../../calibration/fresh/corpus.json"),
        labels: include_str!("../../calibration/fresh/labels.json"),
    },
    Set {
        name: "original",
        corpus: include_str!("../../calibration/corpus.json"),
        labels: include_str!("../../calibration/labels.json"),
    },
];

/// Kinds of result that carry a confidence, in display order.
pub const KINDS: [&str; 3] = ["dead_code", "audit", "architecture"];

/// Levels in display order; `not reported` is a label the current build no
/// longer reports.
const LEVELS: [&str; 4] = ["high", "medium", "low", "not reported"];

/// Roles a set can have.
#[cfg(test)]
const ROLES: [&str; 3] = ["development", "calibration", "sealed"];

/// Position of a level in [`LEVELS`]; lower is more confident.
pub fn rank(level: &str) -> usize {
    LEVELS
        .iter()
        .position(|l| *l == level)
        .unwrap_or(LEVELS.len())
}

/// How the sets may be used; parsed from `calibration/protocol.json`.
#[derive(Debug, Deserialize)]
pub struct Protocol {
    pub gates: Gates,
    pub fixtures: FixtureRules,
    pub sets: Vec<SetEntry>,
}

/// Gates per level; only `high` has one.
#[derive(Debug, Deserialize)]
pub struct Gates {
    pub high: Gate,
}

/// What the fixtures must reach.
#[derive(Debug, Deserialize)]
pub struct FixtureRules {
    pub min_dead_code_recall: f64,
}

/// One set's role and history.
#[derive(Debug, Deserialize)]
pub struct SetEntry {
    pub name: String,
    pub role: String,
    pub about: String,
    pub labelled_with: String,
    pub exposure: Vec<Exposure>,
}

/// One time a set's labels were read.
#[derive(Debug, Deserialize)]
pub struct Exposure {
    pub date: String,
    pub version: String,
    /// Why; for people reading the protocol.
    #[allow(dead_code)]
    pub purpose: String,
    pub details_used_for_tuning: bool,
}

/// The shipped protocol.
pub fn protocol() -> &'static Protocol {
    static PROTOCOL: OnceLock<Protocol> = OnceLock::new();
    PROTOCOL.get_or_init(|| {
        serde_json::from_str(include_str!("../../calibration/protocol.json"))
            .expect("calibration protocol is valid")
    })
}

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
    /// The audit check, e.g. `secret`.
    #[serde(default)]
    pub check: Option<String>,
    /// The level CodeDna gave it when it was labelled; `not reported` for a
    /// `no_finding` sample.
    pub confidence: String,
    /// `correct`, `incorrect` or `unsure`. For a `no_finding` sample,
    /// `incorrect` means the file should have been reported.
    pub verdict: String,
    /// `finding` (CodeDna reported it) or `no_finding` (a randomly sampled
    /// file it did not report).
    #[serde(default = "finding")]
    pub sample: String,
    /// For architecture: the layers CodeDna reported when it was labelled,
    /// as [`fingerprint`] writes them. When this build reports something
    /// else, the label is stale and is not scored.
    #[serde(default)]
    pub output: Option<String>,
}

fn finding() -> String {
    "finding".to_string()
}

impl Label {
    /// Identifies the label within its set.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.kind,
            self.repo,
            normal(&self.file),
            self.line.map(|l| l.to_string()).unwrap_or_default(),
            self.check.as_deref().unwrap_or(""),
            self.sample
        )
    }

    fn no_finding(&self) -> bool {
        self.sample == "no_finding"
    }

    /// What the label measures: the language for dead code, the check for
    /// the audit.
    fn category(&self) -> String {
        match self.kind.as_str() {
            "dead_code" => crate::language_detector::detect_language(Path::new(&self.file))
                .map(|l| l.to_string())
                .unwrap_or_else(|| "other".to_string()),
            "audit" => self.check.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }
}

/// Precision of one level.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelStat {
    pub level: String,
    pub correct: usize,
    /// Labelled `correct` or `incorrect`; `unsure` is left out.
    pub checked: usize,
    /// Pooled over every label (micro average).
    pub precision: f64,
    /// 95% Wilson interval.
    pub low: f64,
    pub high: f64,
    /// Repositories the checked labels come from.
    pub repositories: usize,
    /// Mean of each repository's precision (macro average).
    pub macro_precision: f64,
    /// 95% interval when whole repositories are resampled.
    pub bootstrap_low: f64,
    pub bootstrap_high: f64,
    /// Release gate: `validated`, `unvalidated` or `insufficient_data` for
    /// `high`; `not_gated` for the other levels.
    pub gate: String,
    /// Results this build reports at the level (measured runs only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported: Option<usize>,
    /// Share of those that are labelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<f64>,
}

/// Precision if only results at or above a level were kept.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Selective {
    pub at_least: String,
    pub correct: usize,
    pub checked: usize,
    pub precision: f64,
    /// Share of the checked labels left out.
    pub abstention: f64,
    /// Share of the correct labels still kept: what raising the threshold
    /// costs in true findings.
    pub kept_correct: f64,
}

/// Precision for one language or check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Category {
    pub name: String,
    pub correct: usize,
    pub checked: usize,
    pub precision: f64,
    pub false_positives: usize,
}

/// Recall estimated from randomly sampled files that were not reported.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recall {
    /// `estimated` or `insufficient_data`.
    pub status: String,
    /// `no_finding` samples labelled.
    pub sampled: usize,
    /// How many of them should have been reported.
    pub missed: usize,
    /// Files this build did not report that the samples stand for
    /// (measured runs only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f1: Option<f64>,
}

/// Precision per level for one kind of result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KindStat {
    pub kind: String,
    pub levels: Vec<LevelStat>,
    pub unsure: usize,
    pub selective: Vec<Selective>,
    /// Per language (dead code) or check (audit).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<Category>,
    /// Dead code only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall: Option<Recall>,
    /// Estimated false positives per 1,000 analyzed files (measured runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fp_per_1000_files: Option<f64>,
    /// Results this build declined to call: dead-code candidates whose
    /// status is unknown, architectures it marked unknown (measured runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstained: Option<usize>,
    /// Labels about an output this build no longer gives; not scored.
    #[serde(skip_serializing_if = "is_zero")]
    pub stale: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The whole measurement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calibration {
    /// Which labelled set: `third`, `fresh` or `original`.
    pub set: String,
    /// `development`, `calibration` or `sealed`.
    pub role: String,
    /// What the set is and how it has been used.
    pub about: String,
    /// `published` (the labels as recorded) or `measured` (this build, re-run).
    pub source: String,
    pub repositories: usize,
    /// Files analyzed (measured runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
    pub labels: usize,
    pub kinds: Vec<KindStat>,
    /// Repositories missing from `--corpus DIR`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Stale labels, by key. Never listed for a sealed set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<String>,
    /// Reported results with no label. Never given for a sealed set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_findings: Option<usize>,
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

    fn entry(&self) -> &'static SetEntry {
        protocol()
            .sets
            .iter()
            .find(|e| e.name == self.name)
            .expect("every set is in the protocol")
    }

    /// `development`, `calibration` or `sealed`.
    pub fn role(&self) -> &'static str {
        &self.entry().role
    }

    pub fn sealed(&self) -> bool {
        self.role() == "sealed"
    }

    /// What the set is and how it has been used.
    pub fn about(&self) -> &'static str {
        &self.entry().about
    }

    /// One line on when it was labelled and how often its labels were read.
    pub fn history(&self) -> String {
        let e = self.entry();
        let tuned = e
            .exposure
            .iter()
            .filter(|x| x.details_used_for_tuning)
            .count();
        let last = e
            .exposure
            .last()
            .map(|x| format!("; last read {} with {}", x.date, x.version))
            .unwrap_or_default();
        format!(
            "labelled with {}; read {} time{}, {} used for tuning{last}",
            e.labelled_with,
            e.exposure.len(),
            if e.exposure.len() == 1 { "" } else { "s" },
            tuned
        )
    }
}

/// The sets named `name` with role `role`, or every set.
pub fn sets(name: Option<&str>, role: Option<&str>) -> Vec<&'static Set> {
    SETS.iter()
        .filter(|s| name.is_none_or(|n| n == s.name))
        .filter(|s| role.is_none_or(|r| r == s.role()))
        .collect()
}

/// One label's outcome under some build.
struct Outcome<'a> {
    label: &'a Label,
    /// Level reported, or `not reported`.
    level: String,
    /// Verdict on what was reported.
    verdict: &'a str,
}

/// Precision per level, using the level each label was recorded at.
pub fn published(set: &Set) -> Calibration {
    let labels = set.labels();
    let mut outcomes = Vec::new();
    let mut samples = Vec::new();
    for label in &labels {
        if label.no_finding() {
            samples.push(label);
        } else {
            outcomes.push(Outcome {
                label,
                level: label.confidence.clone(),
                verdict: &label.verdict,
            });
        }
    }
    let scored = Scored {
        outcomes,
        samples,
        stale: Vec::new(),
        measured: None,
    };
    scored.calibration(
        set,
        "published",
        set.corpus().len(),
        labels.len(),
        Vec::new(),
    )
}

/// This build re-run on a set's repositories.
pub struct Run {
    pub set: &'static Set,
    pub labels: Vec<Label>,
    pub now: BTreeMap<String, Current>,
    pub missing: Vec<String>,
}

/// Re-run this build on the repositories cloned under `dir` (one folder per
/// repository, named as in the corpus).
pub fn run(set: &'static Set, dir: &Path) -> Run {
    let mut missing = Vec::new();
    let mut now = BTreeMap::new();
    for repo in set.corpus() {
        let root = dir.join(&repo.repo);
        if !root.is_dir() {
            missing.push(repo.repo.clone());
            continue;
        }
        eprintln!("Measuring {}…", repo.repo);
        now.insert(repo.repo.clone(), Current::of(&root));
    }
    Run {
        set,
        labels: set.labels(),
        now,
        missing,
    }
}

/// What this build reports for one repository.
pub struct Current {
    /// Dead-code candidates: normalized path → (level, status unknown).
    pub dead: BTreeMap<String, (Confidence, bool)>,
    pub audit: Vec<crate::audit::Finding>,
    pub architecture: Confidence,
    /// [`fingerprint`] of the architecture.
    pub layers: String,
    /// Files analyzed.
    pub files: usize,
    /// Files whose imports were followed and that were not reported dead:
    /// what a `no_finding` dead-code sample is drawn from.
    pub frame: BTreeSet<String>,
}

/// The layers of an architecture, sorted: `"Data,Routes,UI"`, `"none"` or
/// `"unknown"`.
pub fn fingerprint(insight: &crate::architecture::ArchitectureInsight) -> String {
    if insight.unknown.is_some() {
        return "unknown".to_string();
    }
    let mut names: Vec<&str> = insight.layers.iter().map(|l| l.name.as_str()).collect();
    if names.is_empty() {
        return "none".to_string();
    }
    names.sort_unstable();
    names.join(",")
}

impl Current {
    pub fn of(root: &Path) -> Self {
        let result = crate::analysis::analyze(root);
        let dead: BTreeMap<String, (Confidence, bool)> = result
            .dead_code_details
            .iter()
            .map(|c| {
                (
                    normal(&c.file.to_string_lossy()),
                    (
                        c.confidence,
                        c.status == crate::dead_code_detector::DeadStatus::Unknown,
                    ),
                )
            })
            .collect();
        let frame = result
            .dependency_graph
            .keys()
            .map(|f| normal(&f.to_string_lossy()))
            .filter(|f| !dead.contains_key(f))
            .collect();
        Current {
            dead,
            audit: crate::audit::scan(root, None).findings,
            architecture: result.architecture_details.confidence,
            layers: fingerprint(&result.architecture_details),
            files: result.file_breakdown.len(),
            frame,
        }
    }

    /// The level this build gives the labelled result, or `None` when it no
    /// longer reports it.
    pub fn level(&self, label: &Label) -> Option<&'static str> {
        let file = normal(&label.file);
        let found = match label.kind.as_str() {
            "dead_code" => self.dead.get(&file).map(|d| d.0),
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

    /// Whether an architecture label is about an output this build no
    /// longer gives.
    pub fn stale(&self, label: &Label) -> bool {
        label.kind == "architecture" && label.output.as_ref().is_some_and(|o| *o != self.layers)
    }

    /// How many results this build reports, per kind and level.
    fn reported(&self, kind: &str, level: &str) -> usize {
        match kind {
            "dead_code" => self.dead.values().filter(|d| d.0.as_str() == level).count(),
            "audit" => self
                .audit
                .iter()
                .filter(|f| f.confidence.as_str() == level)
                .count(),
            _ => usize::from(self.architecture.as_str() == level),
        }
    }

    fn abstained(&self, kind: &str) -> Option<usize> {
        match kind {
            "dead_code" => Some(self.dead.values().filter(|d| d.1).count()),
            "architecture" => Some(usize::from(self.layers == "unknown")),
            _ => None,
        }
    }
}

impl Run {
    /// The level each label has under this build: `Some(level)` (possibly
    /// `not reported`), `Some("stale")`, or `None` when its repository is
    /// missing.
    pub fn level_of(&self, label: &Label) -> Option<&'static str> {
        let current = self.now.get(&label.repo)?;
        if current.stale(label) {
            return Some("stale");
        }
        Some(current.level(label).unwrap_or("not reported"))
    }

    /// Score the labels against what this build reports now.
    pub fn calibration(&self) -> Calibration {
        let mut outcomes = Vec::new();
        let mut samples = Vec::new();
        let mut stale = Vec::new();
        let mut counted = 0;
        for label in &self.labels {
            let Some(level) = self.level_of(label) else {
                continue;
            };
            counted += 1;
            if level == "stale" {
                stale.push(label);
                continue;
            }
            if label.no_finding() {
                if level == "not reported" {
                    samples.push(label);
                } else {
                    // Reported now: a finding, right if the file should
                    // have been reported.
                    let verdict = match label.verdict.as_str() {
                        "incorrect" => "correct",
                        "correct" => "incorrect",
                        other => other,
                    };
                    outcomes.push(Outcome {
                        label,
                        level: level.to_string(),
                        verdict,
                    });
                }
            } else {
                outcomes.push(Outcome {
                    label,
                    level: level.to_string(),
                    verdict: &label.verdict,
                });
            }
        }
        let scored = Scored {
            outcomes,
            samples,
            stale,
            measured: Some(self),
        };
        let mut c = scored.calibration(
            self.set,
            "measured",
            self.now.len(),
            counted,
            self.missing.clone(),
        );
        c.files = Some(self.now.values().map(|n| n.files).sum());
        if !self.set.sealed() {
            c.new_findings = Some(self.unlabelled().len());
        }
        c
    }

    /// Reported dead-code and audit results that no label is about, as
    /// label templates.
    pub fn unlabelled(&self) -> Vec<Label> {
        let keys: BTreeSet<String> = self.labels.iter().map(|l| l.key()).collect();
        let mut out = Vec::new();
        for (repo, current) in &self.now {
            for (file, (confidence, _)) in &current.dead {
                let label = template("dead_code", repo, file, None, None, confidence.as_str());
                if !keys.contains(&label.key()) {
                    out.push(label);
                }
            }
            for f in &current.audit {
                let label = template(
                    "audit",
                    repo,
                    &normal(&f.file),
                    f.line,
                    Some(f.check),
                    f.confidence.as_str(),
                );
                // An audit label without a line matches any line.
                let loose = Label {
                    line: None,
                    ..label.clone()
                };
                if !keys.contains(&label.key()) && !keys.contains(&loose.key()) {
                    out.push(label);
                }
            }
        }
        out
    }

    /// A seeded labelling queue: up to `n` unlabelled findings per kind and
    /// `n` files that were not reported dead, for `no_finding` labels.
    pub fn queue(&self, n: usize, seed: u64) -> Vec<Label> {
        let mut rng = stats::XorShift(seed.max(1));
        let mut pick = |mut pool: Vec<Label>| {
            let mut chosen = Vec::new();
            while chosen.len() < n && !pool.is_empty() {
                let i = rng.below(pool.len());
                chosen.push(pool.swap_remove(i));
            }
            chosen.sort_by_key(|l| l.key());
            chosen
        };
        let unlabelled = self.unlabelled();
        let (dead, audit): (Vec<Label>, Vec<Label>) =
            unlabelled.into_iter().partition(|l| l.kind == "dead_code");
        let labelled: BTreeSet<String> = self.labels.iter().map(|l| l.key()).collect();
        let frame: Vec<Label> = self
            .now
            .iter()
            .flat_map(|(repo, current)| {
                current.frame.iter().map(move |file| Label {
                    sample: "no_finding".to_string(),
                    ..template("dead_code", repo, file, None, None, "not reported")
                })
            })
            .filter(|l| !labelled.contains(&l.key()))
            .collect();
        let mut out = pick(dead);
        out.extend(pick(audit));
        out.extend(pick(frame));
        out
    }
}

fn template(
    kind: &str,
    repo: &str,
    file: &str,
    line: Option<usize>,
    check: Option<&str>,
    confidence: &str,
) -> Label {
    Label {
        kind: kind.to_string(),
        repo: repo.to_string(),
        file: file.to_string(),
        line,
        check: check.map(str::to_string),
        confidence: confidence.to_string(),
        verdict: String::new(),
        sample: finding(),
        output: None,
    }
}

/// `./src/a.ts` → `src/a.ts`, with forward slashes.
pub fn normal(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.strip_prefix("./").unwrap_or(&path).to_string()
}

/// Outcomes ready to count.
struct Scored<'a> {
    outcomes: Vec<Outcome<'a>>,
    /// `no_finding` samples still not reported.
    samples: Vec<&'a Label>,
    stale: Vec<&'a Label>,
    measured: Option<&'a Run>,
}

fn verdict_counts<'b>(outcomes: impl Iterator<Item = &'b Outcome<'b>>) -> (usize, usize) {
    let (mut correct, mut checked) = (0, 0);
    for o in outcomes {
        match o.verdict {
            "correct" => {
                correct += 1;
                checked += 1;
            }
            "incorrect" => checked += 1,
            _ => {}
        }
    }
    (correct, checked)
}

impl<'a> Scored<'a> {
    fn calibration(
        &self,
        set: &Set,
        source: &str,
        repositories: usize,
        labels: usize,
        missing: Vec<String>,
    ) -> Calibration {
        let kinds = KINDS.iter().map(|&kind| self.kind(kind)).collect();
        Calibration {
            set: set.name.to_string(),
            role: set.role().to_string(),
            about: set.about().to_string(),
            source: source.to_string(),
            repositories,
            files: None,
            labels,
            kinds,
            missing,
            stale: if set.sealed() {
                Vec::new()
            } else {
                self.stale.iter().map(|l| l.key()).collect()
            },
            new_findings: None,
        }
    }

    fn kind(&self, kind: &str) -> KindStat {
        let of_kind: Vec<&Outcome> = self
            .outcomes
            .iter()
            .filter(|o| o.label.kind == kind)
            .collect();
        let unsure = of_kind
            .iter()
            .filter(|o| !matches!(o.verdict, "correct" | "incorrect"))
            .count();
        let levels: Vec<LevelStat> = LEVELS
            .iter()
            .filter_map(|&level| self.level(kind, level, &of_kind))
            .collect();

        // Selective precision: keep only results at or above a level.
        let (all_correct, all_checked) = verdict_counts(of_kind.iter().copied());
        let selective = LEVELS[..3]
            .iter()
            .map(|&at_least| {
                let (correct, checked) = verdict_counts(
                    of_kind
                        .iter()
                        .copied()
                        .filter(|o| rank(&o.level) <= rank(at_least)),
                );
                Selective {
                    at_least: at_least.to_string(),
                    correct,
                    checked,
                    precision: ratio(correct, checked),
                    abstention: 1.0 - ratio(checked, all_checked),
                    kept_correct: ratio(correct, all_correct),
                }
            })
            .collect();

        let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for o in of_kind.iter().filter(|o| o.level != "not reported") {
            let name = o.label.category();
            if name.is_empty() {
                continue;
            }
            let entry = by_category.entry(name).or_default();
            match o.verdict {
                "correct" => {
                    entry.0 += 1;
                    entry.1 += 1;
                }
                "incorrect" => entry.1 += 1,
                _ => {}
            }
        }
        let categories = by_category
            .into_iter()
            .filter(|(_, (_, checked))| *checked > 0)
            .map(|(name, (correct, checked))| Category {
                name,
                correct,
                checked,
                precision: ratio(correct, checked),
                false_positives: checked - correct,
            })
            .collect();

        let recall = (kind == "dead_code").then(|| self.recall(&levels));

        let (fp_per_1000_files, abstained) = match self.measured {
            Some(run) => {
                let files: usize = run.now.values().map(|n| n.files).sum();
                let fp: f64 = levels
                    .iter()
                    .filter_map(|l| Some(l.reported? as f64 * (1.0 - l.precision)))
                    .sum();
                let fp = (kind != "architecture" && files > 0).then(|| fp * 1000.0 / files as f64);
                let abstained = run
                    .now
                    .values()
                    .map(|n| n.abstained(kind))
                    .sum::<Option<usize>>();
                (fp, abstained)
            }
            None => (None, None),
        };

        KindStat {
            kind: kind.to_string(),
            levels,
            unsure,
            selective,
            categories,
            recall,
            fp_per_1000_files,
            abstained,
            stale: self.stale.iter().filter(|l| l.kind == kind).count(),
        }
    }

    fn level(&self, kind: &str, level: &str, of_kind: &[&Outcome]) -> Option<LevelStat> {
        let at: Vec<&Outcome> = of_kind
            .iter()
            .copied()
            .filter(|o| o.level == level)
            .collect();
        let (correct, checked) = verdict_counts(at.iter().copied());
        if checked == 0 {
            return None;
        }
        let mut per_repo: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for o in &at {
            let entry = per_repo.entry(&o.label.repo).or_default();
            match o.verdict {
                "correct" => {
                    entry.0 += 1;
                    entry.1 += 1;
                }
                "incorrect" => entry.1 += 1,
                _ => {}
            }
        }
        let per_repo: Vec<(usize, usize)> = per_repo.into_values().filter(|r| r.1 > 0).collect();
        let (low, high) = stats::wilson(correct, checked);
        let (bootstrap_low, bootstrap_high) = stats::bootstrap(&per_repo);
        let gate = if level == "high" {
            protocol().gates.high.check(&Evidence {
                correct,
                checked,
                repositories: per_repo.len(),
                wilson_low: low,
                bootstrap_low,
            })
        } else {
            GateStatus::NotGated
        };
        let reported = self
            .measured
            .filter(|_| level != "not reported")
            .map(|run| run.now.values().map(|n| n.reported(kind, level)).sum());
        Some(LevelStat {
            level: level.to_string(),
            correct,
            checked,
            precision: ratio(correct, checked),
            low,
            high,
            repositories: per_repo.len(),
            macro_precision: stats::macro_precision(&per_repo),
            bootstrap_low,
            bootstrap_high,
            gate: gate.as_str().to_string(),
            reported,
            coverage: reported
                .filter(|&r| r > 0)
                .map(|r| (checked as f64 / r as f64).min(1.0)),
        })
    }

    fn recall(&self, levels: &[LevelStat]) -> Recall {
        let samples: Vec<&&Label> = self
            .samples
            .iter()
            .filter(|l| {
                l.kind == "dead_code" && matches!(l.verdict.as_str(), "correct" | "incorrect")
            })
            .collect();
        let sampled = samples.len();
        let missed = samples.iter().filter(|l| l.verdict == "incorrect").count();
        let frame = self
            .measured
            .map(|run| run.now.values().map(|n| n.frame.len()).sum::<usize>());
        let reported: Vec<(usize, f64)> = levels
            .iter()
            .filter_map(|l| Some((l.reported?, l.precision)))
            .collect();
        let estimate =
            frame.and_then(|frame| stats::estimated_recall(&reported, frame, sampled, missed));
        let f1 = estimate.and_then(|r| {
            let total: usize = reported.iter().map(|r| r.0).sum();
            let found: f64 = reported.iter().map(|&(n, p)| n as f64 * p).sum();
            let p = found / total as f64;
            (p + r > 0.0).then(|| 2.0 * p * r / (p + r))
        });
        Recall {
            status: if estimate.is_some() {
                "estimated"
            } else {
                "insufficient_data"
            }
            .to_string(),
            sampled,
            missed,
            frame,
            estimate,
            f1,
        }
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

/// Whether a kind's `high` level passed the release gate on the sealed set,
/// as reports carry it in `analyzer.confidence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfidenceCheck {
    /// `dead_code`, `audit` or `architecture`.
    pub kind: String,
    /// The level checked: `high`.
    pub level: String,
    /// `validated`, `unvalidated` or `insufficient_data`.
    pub status: String,
    /// The sealed set it was measured on.
    pub set: String,
    /// Correct plus incorrect labels at the level.
    pub outcomes: usize,
    /// Repositories they come from.
    pub repositories: usize,
}

/// The release gate for every kind's `high` level, from the sealed set's
/// published labels.
pub fn confidence_checks() -> &'static [ConfidenceCheck] {
    static CHECKS: OnceLock<Vec<ConfidenceCheck>> = OnceLock::new();
    CHECKS.get_or_init(|| {
        let Some(set) = SETS.iter().find(|s| s.sealed()) else {
            return Vec::new();
        };
        let c = published(set);
        c.kinds
            .iter()
            .map(|k| {
                let high = k.levels.iter().find(|l| l.level == "high");
                ConfidenceCheck {
                    kind: k.kind.clone(),
                    level: "high".to_string(),
                    status: high
                        .map(|h| h.gate.clone())
                        .unwrap_or_else(|| GateStatus::InsufficientData.as_str().to_string()),
                    set: set.name.to_string(),
                    outcomes: high.map_or(0, |h| h.checked),
                    repositories: high.map_or(0, |h| h.repositories),
                }
            })
            .collect()
    })
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "dead_code" => "Dead code",
        "audit" => "Audit",
        _ => "Architecture",
    }
}

/// Plain-text table.
pub fn render(c: &Calibration) -> String {
    let mut out = format!(
        "Measured confidence, {} set, {} ({} labels from {} repositories{})\n  {}\n\n",
        c.set,
        c.role,
        c.labels,
        c.repositories,
        if c.source == "measured" {
            ", re-run with this build"
        } else {
            ""
        },
        c.about
    );
    if let Some(set) = SETS.iter().find(|s| s.name == c.set) {
        out.insert_str(out.len() - 1, &format!("  {}\n", set.history()));
    }
    for kind in &c.kinds {
        let name = kind_name(&kind.kind);
        for (i, s) in kind.levels.iter().enumerate() {
            let gate = GateStatus::words(match s.gate.as_str() {
                "validated" => GateStatus::Validated,
                "unvalidated" => GateStatus::Unvalidated,
                "insufficient_data" => GateStatus::InsufficientData,
                _ => GateStatus::NotGated,
            });
            out.push_str(&format!(
                "  {:<13} {:<13} {:>3}/{:<3} {:>4.0}%  ({:.0}–{:.0}%, {} repos){}\n",
                if i == 0 { name } else { "" },
                s.level,
                s.correct,
                s.checked,
                s.precision * 100.0,
                s.low * 100.0,
                s.high * 100.0,
                s.repositories,
                if gate.is_empty() {
                    String::new()
                } else {
                    format!("  gate: {gate}")
                }
            ));
        }
        if kind.unsure > 0 {
            out.push_str(&format!("  {:<13} {} unsure, left out\n", "", kind.unsure));
        }
        if kind.stale > 0 {
            out.push_str(&format!(
                "  {:<13} {} stale (output changed), left out\n",
                "", kind.stale
            ));
        }
        if let Some(r) = &kind.recall {
            match r.estimate {
                Some(e) => out.push_str(&format!(
                    "  {:<13} recall about {:.0}% ({} of {} sampled unreported files missed)\n",
                    "",
                    e * 100.0,
                    r.missed,
                    r.sampled
                )),
                None if c.source == "measured" => out.push_str(&format!(
                    "  {:<13} recall: insufficient data ({} no-finding samples)\n",
                    "", r.sampled
                )),
                None => {}
            }
        }
    }
    let gate = &protocol().gates.high;
    out.push_str(&format!(
        "\nPrecision: of the findings at that level, how many were right. \
         The range is a 95% interval; few findings give a wide one.\n\
         Gate: high is validated at {:.0}% precision or more, with both the 95% \
         interval and the repository bootstrap above {:.0}%, on at least {} \
         labels from {} repositories.\n",
        gate.min_precision * 100.0,
        gate.min_lower_bound * 100.0,
        gate.min_outcomes,
        gate.min_repositories
    ));
    if c.kinds
        .iter()
        .any(|k| k.levels.iter().any(|s| s.level == "not reported"))
    {
        out.push_str(
            "\"not reported\": labelled findings this build no longer reports; \
             a low number there means it dropped false alarms.\n",
        );
    }
    if let Some(n) = c.new_findings {
        out.push_str(&format!(
            "{n} reported results have no label yet; `--queue FILE` samples some to label.\n"
        ));
    }
    if !c.missing.is_empty() {
        out.push_str("\nNot found in the corpus folder; clone each at its pinned commit:\n");
        let corpus = sets(Some(&c.set), None)
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
                assert!(["finding", "no_finding"].contains(&label.sample.as_str()));
            }
            // A held-out set shares no repository with another set.
            for repo in repos {
                assert!(all_repos.insert(repo.clone()), "{repo} is in two sets");
            }
        }
        assert_eq!(sets(Some("fresh"), None).len(), 1);
        assert_eq!(sets(None, None).len(), SETS.len());
        assert_eq!(sets(None, Some("development")).len(), 2);
    }

    #[test]
    fn protocol_covers_every_set_and_seals_one() {
        let p = protocol();
        for set in &SETS {
            assert!(ROLES.contains(&set.role()), "{}", set.name);
            assert!(!set.about().is_empty());
        }
        assert_eq!(p.sets.len(), SETS.len());
        assert_eq!(SETS.iter().filter(|s| s.sealed()).count(), 1);
        // A set whose details were used for tuning is a development set.
        for entry in &p.sets {
            assert!(!entry.exposure.is_empty(), "{}", entry.name);
            assert!(!entry.labelled_with.is_empty());
            for e in &entry.exposure {
                assert!(!e.date.is_empty() && !e.version.is_empty() && !e.purpose.is_empty());
                if e.details_used_for_tuning {
                    assert_eq!(entry.role, "development", "{} was tuned on", entry.name);
                }
            }
        }
        assert!(p.fixtures.min_dead_code_recall > 0.0);
        assert!(p.gates.high.min_outcomes >= 200);
    }

    fn label(kind: &str, repo: &str, verdict: &str) -> Label {
        Label {
            kind: kind.to_string(),
            repo: repo.to_string(),
            file: "src/a.ts".to_string(),
            line: None,
            check: (kind == "audit").then(|| "secret".to_string()),
            confidence: "high".to_string(),
            verdict: verdict.to_string(),
            sample: finding(),
            output: None,
        }
    }

    fn scored<'a>(labels: &'a [Label], levels: &[&str]) -> Scored<'a> {
        Scored {
            outcomes: labels
                .iter()
                .zip(levels)
                .map(|(label, level)| Outcome {
                    label,
                    level: level.to_string(),
                    verdict: &label.verdict,
                })
                .collect(),
            samples: Vec::new(),
            stale: Vec::new(),
            measured: None,
        }
    }

    #[test]
    fn scores_per_kind_and_level() {
        let labels = vec![
            label("dead_code", "r", "correct"),
            label("dead_code", "s", "incorrect"),
            label("dead_code", "r", "unsure"),
            label("audit", "r", "correct"),
        ];
        let c = scored(&labels, &["high", "high", "high", "not reported"]).calibration(
            &SETS[1],
            "published",
            2,
            labels.len(),
            Vec::new(),
        );
        let dead = &c.kinds[0];
        assert_eq!(dead.levels.len(), 1);
        let high = &dead.levels[0];
        assert_eq!((high.correct, high.checked, high.repositories), (1, 2, 2));
        assert_eq!(high.macro_precision, 0.5);
        assert_eq!(high.gate, "insufficient_data");
        assert_eq!(dead.unsure, 1);
        assert_eq!(dead.categories[0].name, "TypeScript");
        assert_eq!(dead.categories[0].false_positives, 1);
        assert_eq!(dead.recall.as_ref().unwrap().status, "insufficient_data");
        assert_eq!(c.kinds[1].levels[0].level, "not reported");
        assert_eq!(c.kinds[1].levels[0].gate, "not_gated");
        assert!(c.kinds[2].levels.is_empty());
        assert_eq!(c.role, "development");
        let text = render(&c);
        assert!(text.contains("Dead code") && text.contains("insufficient data"));
    }

    #[test]
    fn selective_precision_trades_abstention_for_precision() {
        let labels = vec![
            label("dead_code", "r", "correct"),
            label("dead_code", "r", "correct"),
            label("dead_code", "r", "incorrect"),
            label("dead_code", "r", "incorrect"),
        ];
        let c = scored(&labels, &["high", "medium", "low", "not reported"]).calibration(
            &SETS[1],
            "published",
            1,
            4,
            Vec::new(),
        );
        let s = &c.kinds[0].selective;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].at_least.as_str(), s[0].precision), ("high", 1.0));
        assert_eq!(s[0].abstention, 0.75);
        assert_eq!(s[0].kept_correct, 0.5);
        assert_eq!((s[1].correct, s[1].checked, s[1].kept_correct), (2, 2, 1.0));
        assert!((s[2].precision - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(s[2].abstention, 0.25);
    }

    #[test]
    fn labels_default_to_findings_and_key_by_sample() {
        let l: Label = serde_json::from_str(
            r#"{"kind":"dead_code","repo":"r","file":"./a.py","confidence":"low","verdict":"correct"}"#,
        )
        .unwrap();
        assert_eq!(l.sample, "finding");
        assert_eq!(l.key(), "dead_code|r|a.py|||finding");
        assert_eq!(l.category(), "Python");
        assert_eq!(rank("high"), 0);
        assert_eq!(rank("not reported"), 3);
    }

    #[test]
    fn reports_carry_one_check_per_kind() {
        let checks = confidence_checks();
        let kinds: Vec<&str> = checks.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, KINDS);
        for c in checks {
            assert_ne!(c.status, "not_gated");
            assert_eq!(c.set, "third");
        }
    }

    #[test]
    fn paths_compare_without_leading_dot() {
        assert_eq!(normal("./src/a.ts"), "src/a.ts");
        assert_eq!(normal("src\\a.ts"), "src/a.ts");
    }
}
