//! Baselines: accept the findings a project already has, so only new ones
//! are reported.
//!
//! A baseline is a JSON file written by `--write-baseline` and read by
//! `--baseline`. It records audit findings by check, file and message (not
//! line, so they survive edits elsewhere in the file) and dead-code
//! candidates by file. Each entry may carry a `reason`, which is kept when
//! the baseline is written again.
//!
//! A baseline is only ever read from a path the user names: a repository
//! cannot hide its own findings by shipping one.

use crate::audit::Finding;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Version written to new baselines.
pub const VERSION: u32 = 1;
/// Largest baseline file read.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// An accepted audit finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub check: String,
    pub file: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// An accepted dead-code candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadCodeEntry {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Findings accepted as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub version: u32,
    #[serde(default)]
    pub audit: Vec<AuditEntry>,
    #[serde(default)]
    pub dead_code: Vec<DeadCodeEntry>,
}

/// What applying a baseline did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Applied {
    /// Findings hidden because the baseline accepts them.
    pub hidden: usize,
    /// Baseline entries that matched nothing: fixed, or changed enough to
    /// be reported again. Safe to remove by writing the baseline again.
    pub stale: usize,
}

/// One line saying what a baseline hid and which entries are stale.
/// `what` names one hidden item, e.g. "finding".
pub fn summary(applied: Applied, what: &str) -> String {
    let mut out = format!(
        "  Baseline: {} known {what}{} hidden",
        applied.hidden,
        if applied.hidden == 1 { "" } else { "s" },
    );
    if applied.stale > 0 {
        out.push_str(&format!(
            "; {} entr{} no longer found (write the baseline again to drop {})",
            applied.stale,
            if applied.stale == 1 { "y" } else { "ies" },
            if applied.stale == 1 { "it" } else { "them" },
        ));
    }
    out.push('.');
    out
}

/// Paths are compared without a leading `./` and with `/` separators.
fn key(file: &str) -> String {
    let file = file.replace('\\', "/");
    file.strip_prefix("./").unwrap_or(&file).to_string()
}

impl Baseline {
    /// Read a baseline. A missing file is an error: a gate that silently ran
    /// without its baseline would fail on every accepted finding.
    pub fn load(path: &Path) -> Result<Baseline, String> {
        let shown = crate::safe_text::sanitize(&path.to_string_lossy());
        let meta = std::fs::metadata(path).map_err(|_| format!("Baseline not found: '{shown}'"))?;
        if meta.len() > MAX_BYTES {
            return Err(format!("Baseline '{shown}' is too large"));
        }
        let text =
            std::fs::read_to_string(path).map_err(|_| format!("Can't read baseline '{shown}'"))?;
        let baseline: Baseline = serde_json::from_str(&text)
            .map_err(|e| format!("Baseline '{shown}' is not valid: {e}"))?;
        if baseline.version != VERSION {
            return Err(format!(
                "Baseline '{shown}' has version {}; this CodeDna reads version {VERSION}",
                baseline.version
            ));
        }
        Ok(baseline)
    }

    /// Read `path` if it exists, so writing one section keeps the other.
    pub fn load_or_default(path: &Path) -> Result<Baseline, String> {
        if path.exists() {
            Baseline::load(path)
        } else {
            Ok(Baseline {
                version: VERSION,
                ..Baseline::default()
            })
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let shown = crate::safe_text::sanitize(&path.to_string_lossy());
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(path, text).map_err(|_| format!("Can't write baseline '{shown}'"))
    }

    /// Replace the audit section with `findings`, keeping the reasons of
    /// entries that are still there.
    pub fn set_audit(&mut self, findings: &[Finding]) {
        let reasons: HashMap<(String, String, String), Option<String>> = self
            .audit
            .drain(..)
            .map(|e| ((e.check, key(&e.file), e.message), e.reason))
            .collect();
        let mut audit: Vec<AuditEntry> = findings
            .iter()
            .map(|f| {
                let id = (f.check.to_string(), key(&f.file), f.message.clone());
                AuditEntry {
                    reason: reasons.get(&id).cloned().flatten(),
                    check: id.0,
                    file: id.1,
                    message: id.2,
                }
            })
            .collect();
        audit.sort_by(|a, b| (&a.file, &a.check, &a.message).cmp(&(&b.file, &b.check, &b.message)));
        self.audit = audit;
    }

    /// Replace the dead-code section with `files`, keeping reasons.
    pub fn set_dead_code<'a>(&mut self, files: impl IntoIterator<Item = &'a str>) {
        let reasons: HashMap<String, Option<String>> = self
            .dead_code
            .drain(..)
            .map(|e| (key(&e.file), e.reason))
            .collect();
        let mut dead: Vec<DeadCodeEntry> = files
            .into_iter()
            .map(|f| {
                let file = key(f);
                DeadCodeEntry {
                    reason: reasons.get(&file).cloned().flatten(),
                    file,
                }
            })
            .collect();
        dead.sort_by(|a, b| a.file.cmp(&b.file));
        dead.dedup_by(|a, b| a.file == b.file);
        self.dead_code = dead;
    }

    /// Drop the findings the baseline accepts. Each entry accepts one
    /// finding, so a second identical finding in the same file is new.
    pub fn filter_audit(&self, findings: &mut Vec<Finding>) -> Applied {
        let mut budget: HashMap<(String, String, String), usize> = HashMap::new();
        for e in &self.audit {
            *budget
                .entry((e.check.clone(), key(&e.file), e.message.clone()))
                .or_default() += 1;
        }
        let before = findings.len();
        findings.retain(|f| {
            let id = (f.check.to_string(), key(&f.file), f.message.clone());
            match budget.get_mut(&id) {
                Some(left) if *left > 0 => {
                    *left -= 1;
                    false
                }
                _ => true,
            }
        });
        Applied {
            hidden: before - findings.len(),
            stale: budget.values().sum(),
        }
    }

    /// Whether the baseline accepts `file` as dead code.
    pub fn accepts_dead(&self, file: &str) -> bool {
        let file = key(file);
        self.dead_code.iter().any(|e| key(&e.file) == file)
    }

    /// Dead-code entries that match none of `files`.
    pub fn stale_dead<'a>(&self, files: impl IntoIterator<Item = &'a str>) -> usize {
        let found: std::collections::HashSet<String> = files.into_iter().map(key).collect();
        self.dead_code
            .iter()
            .filter(|e| !found.contains(&key(&e.file)))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::Confidence;
    use crate::audit::Severity;

    fn finding(check: &'static str, file: &str, message: &str) -> Finding {
        Finding {
            severity: Severity::High,
            confidence: Confidence::High,
            check,
            file: file.to_string(),
            line: Some(3),
            message: message.to_string(),
            context: None,
            evidence: crate::audit::Evidence {
                strength: crate::audit::Strength::Strong,
                signals: Vec::new(),
            },
            dependency: None,
        }
    }

    #[test]
    fn hides_accepted_findings_and_counts_stale_entries() {
        let mut baseline = Baseline {
            version: VERSION,
            ..Baseline::default()
        };
        baseline.set_audit(&[
            finding("secret", "./src/a.py", "Possible key"),
            finding("binary", "./tool.exe", "Executable"),
        ]);
        assert_eq!(baseline.audit[0].file, "src/a.py");
        let mut findings = vec![
            finding("secret", "./src/a.py", "Possible key"),
            finding("secret", "./src/a.py", "Possible key"),
            finding("secret", "./src/b.py", "Possible key"),
        ];
        let applied = baseline.filter_audit(&mut findings);
        assert_eq!(
            applied,
            Applied {
                hidden: 1,
                stale: 1
            }
        );
        let left: Vec<&str> = findings.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(left, ["./src/a.py", "./src/b.py"]);
    }

    #[test]
    fn keeps_reasons_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("baseline.json");
        let mut baseline = Baseline::load_or_default(&path).unwrap();
        baseline.set_audit(&[finding("secret", "src/a.py", "Possible key")]);
        baseline.set_dead_code(["./scripts/old.js", "./scripts/old.js"]);
        baseline.audit[0].reason = Some("test key".to_string());
        baseline.dead_code[0].reason = Some("run by hand".to_string());
        baseline.save(&path).unwrap();

        let mut again = Baseline::load(&path).unwrap();
        assert_eq!(again, baseline);
        again.set_audit(&[
            finding("secret", "./src/a.py", "Possible key"),
            finding("binary", "x.exe", "Executable"),
        ]);
        again.set_dead_code(["scripts/old.js"]);
        assert_eq!(again.audit[0].reason.as_deref(), Some("test key"));
        assert_eq!(again.audit[1].reason, None);
        assert_eq!(again.dead_code[0].reason.as_deref(), Some("run by hand"));
        assert!(again.accepts_dead("./scripts/old.js"));
        assert_eq!(again.stale_dead(["./src/x.js"]), 1);
    }

    #[test]
    fn refuses_missing_or_unknown_baselines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.json");
        assert!(Baseline::load(&path).unwrap_err().contains("not found"));
        std::fs::write(&path, r#"{"version": 9}"#).unwrap();
        assert!(Baseline::load(&path).unwrap_err().contains("version 9"));
        std::fs::write(&path, "not json").unwrap();
        assert!(Baseline::load(&path).unwrap_err().contains("not valid"));
    }
}
