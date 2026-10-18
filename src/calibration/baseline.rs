//! A recorded measurement of the development sets, so a change that makes a
//! labelled result worse is caught.
//!
//! For every label it stores the level this build gives the result. A later
//! build regresses when a result labelled `correct` drops a level (or stops
//! being reported), when one labelled `incorrect` rises a level, or when an
//! architecture label goes stale. New labels and changed counts are shown but
//! do not fail the check.

use super::{rank, Run};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    pub sets: Vec<String>,
    pub labels: Vec<Entry>,
    pub repositories: BTreeMap<String, RepoCounts>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub set: String,
    pub key: String,
    /// `high`, `medium`, `low`, `not reported` or `stale`.
    pub level: String,
    pub verdict: String,
}

/// What this build reported for one repository, per level.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RepoCounts {
    pub files: usize,
    pub dead_code: BTreeMap<String, usize>,
    pub audit: BTreeMap<String, usize>,
    /// `layers|level`.
    pub architecture: String,
}

/// Record the runs.
pub fn record(runs: &[Run]) -> Baseline {
    let mut labels = Vec::new();
    let mut repositories = BTreeMap::new();
    for run in runs {
        for label in &run.labels {
            let Some(level) = run.level_of(label) else {
                continue;
            };
            labels.push(Entry {
                set: run.set.name.to_string(),
                key: label.key(),
                level: level.to_string(),
                verdict: label.verdict.clone(),
            });
        }
        for (repo, current) in &run.now {
            let mut counts = RepoCounts {
                files: current.files,
                architecture: format!("{}|{}", current.layers, current.architecture.as_str()),
                ..RepoCounts::default()
            };
            for (confidence, _) in current.dead.values() {
                *counts
                    .dead_code
                    .entry(confidence.as_str().to_string())
                    .or_default() += 1;
            }
            for f in &current.audit {
                *counts
                    .audit
                    .entry(f.confidence.as_str().to_string())
                    .or_default() += 1;
            }
            repositories.insert(repo.clone(), counts);
        }
    }
    Baseline {
        version: VERSION,
        sets: runs.iter().map(|r| r.set.name.to_string()).collect(),
        labels,
        repositories,
    }
}

/// Differences from a baseline.
#[derive(Debug, Default)]
pub struct Comparison {
    /// Each fails the check.
    pub regressions: Vec<String>,
    /// Shown, not failing.
    pub improvements: Vec<String>,
    pub notes: Vec<String>,
}

/// Compare a fresh measurement with a recorded one.
pub fn compare(old: &Baseline, new: &Baseline, missing: &[String]) -> Comparison {
    let mut c = Comparison::default();
    for repo in missing {
        if old.repositories.contains_key(repo) {
            c.regressions
                .push(format!("{repo}: not found in the corpus folder"));
        }
    }
    let before: BTreeMap<(&str, &str), &Entry> = old
        .labels
        .iter()
        .map(|e| ((e.set.as_str(), e.key.as_str()), e))
        .collect();
    for e in &new.labels {
        let Some(was) = before.get(&(e.set.as_str(), e.key.as_str())) else {
            c.notes.push(format!("{} {}: new label", e.set, e.key));
            continue;
        };
        if was.level == e.level {
            continue;
        }
        let what = format!("{} {}: {} → {}", e.set, e.key, was.level, e.level);
        if e.level == "stale" {
            c.regressions
                .push(format!("{what} (output changed; re-check the label)"));
            continue;
        }
        if was.level == "stale" {
            c.notes.push(what);
            continue;
        }
        let (from, to) = (rank(&was.level), rank(&e.level));
        match e.verdict.as_str() {
            "correct" if to > from => c.regressions.push(format!("{what} (labelled correct)")),
            "incorrect" if to < from => c.regressions.push(format!("{what} (labelled incorrect)")),
            "correct" | "incorrect" => c.improvements.push(what),
            _ => c.notes.push(what),
        }
    }
    for (repo, counts) in &new.repositories {
        match old.repositories.get(repo) {
            Some(was) if was != counts => {
                if was.architecture != counts.architecture {
                    c.notes.push(format!(
                        "{repo}: architecture {} → {}",
                        was.architecture, counts.architecture
                    ));
                }
                if was.dead_code != counts.dead_code || was.audit != counts.audit {
                    c.notes.push(format!(
                        "{repo}: dead code {:?} → {:?}, audit {:?} → {:?}",
                        was.dead_code, counts.dead_code, was.audit, counts.audit
                    ));
                }
            }
            None => c.notes.push(format!("{repo}: not in the baseline")),
            _ => {}
        }
    }
    c
}

/// Plain-text result.
pub fn render(c: &Comparison, against: &str) -> String {
    let mut out = String::new();
    if c.regressions.is_empty() {
        out.push_str(&format!("No regressions against {against}.\n"));
    } else {
        out.push_str(&format!(
            "{} regression{} against {against}:\n",
            c.regressions.len(),
            if c.regressions.len() == 1 { "" } else { "s" }
        ));
        for r in &c.regressions {
            out.push_str(&format!("  {r}\n"));
        }
    }
    if !c.improvements.is_empty() {
        out.push_str(&format!("\n{} improvements:\n", c.improvements.len()));
        for r in &c.improvements {
            out.push_str(&format!("  {r}\n"));
        }
    }
    if !c.notes.is_empty() {
        out.push_str(&format!("\n{} other changes:\n", c.notes.len()));
        for r in &c.notes {
            out.push_str(&format!("  {r}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, level: &str, verdict: &str) -> Entry {
        Entry {
            set: "fresh".to_string(),
            key: key.to_string(),
            level: level.to_string(),
            verdict: verdict.to_string(),
        }
    }

    fn baseline(labels: Vec<Entry>) -> Baseline {
        Baseline {
            version: VERSION,
            sets: vec!["fresh".to_string()],
            labels,
            repositories: BTreeMap::from([("r".to_string(), RepoCounts::default())]),
        }
    }

    #[test]
    fn flags_correct_labels_that_drop_and_incorrect_ones_that_rise() {
        let old = baseline(vec![
            entry("a", "high", "correct"),
            entry("b", "low", "incorrect"),
            entry("c", "medium", "correct"),
            entry("d", "medium", "incorrect"),
            entry("e", "high", "correct"),
            entry("f", "low", "unsure"),
        ]);
        let new = baseline(vec![
            entry("a", "not reported", "correct"),
            entry("b", "high", "incorrect"),
            entry("c", "high", "correct"),
            entry("d", "not reported", "incorrect"),
            entry("e", "stale", "correct"),
            entry("f", "high", "unsure"),
            entry("g", "low", "correct"),
        ]);
        let c = compare(&old, &new, &[]);
        assert_eq!(c.regressions.len(), 3, "{:?}", c.regressions);
        assert!(c.regressions[0].contains("fresh a"));
        assert!(c.regressions[1].contains("fresh b"));
        assert!(c.regressions[2].contains("stale"));
        assert_eq!(c.improvements.len(), 2);
        assert_eq!(c.notes.len(), 2);
        assert!(render(&c, "x").contains("3 regressions"));
    }

    #[test]
    fn same_measurement_and_missing_repositories() {
        let b = baseline(vec![entry("a", "high", "correct")]);
        let c = compare(&b, &b, &[]);
        assert!(c.regressions.is_empty());
        assert!(render(&c, "x").starts_with("No regressions against x"));
        let c = compare(&b, &b, &["r".to_string()]);
        assert_eq!(c.regressions.len(), 1);
    }
}
