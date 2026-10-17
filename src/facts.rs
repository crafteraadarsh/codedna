//! Typed dependency facts: one record per dependency the analysis found,
//! saying what kind of dependency it is, how it resolved, how certain it is
//! and where in the source it was seen.
//!
//! The extractors in [`crate::dependency_graph`] emit facts, and
//! [`crate::graph`] builds the plain `file → [files]` graph from the
//! resolved ones, so `dependency_graph` in the report is a projection of
//! `dependencies.facts`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Version of the extraction rules. It goes up when a rule starts or stops
/// producing facts, or a rule id changes meaning, so consumers can tell
/// reports made with different rules apart.
pub const RULES_VERSION: u32 = 1;

/// What kind of dependency a fact records.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// Code loaded when the source runs: `import`, `require`, `import()`,
    /// Python `import`, Rust `mod`.
    RuntimeImport,
    /// Types only, erased at runtime: `import type`, `export type ... from`.
    TypeOnlyImport,
    /// Exports of another module passed on: `export ... from`.
    ReExport,
    /// A file started in a worker thread: `new Worker("./w.js")`.
    WorkerEntry,
    /// A file started in another process: `spawn("tsx", ["job.ts"])`.
    ProcessEntry,
    /// A file referenced by URL: `new URL("./w.ts", import.meta.url)`.
    ResourceReference,
    /// A module a framework loads from configuration or by convention:
    /// Django settings strings, installed apps and app modules.
    ConfiguredEntry,
    /// Something that may be loaded, but whose target is only known at
    /// runtime: ``import(`./pages/${name}`)``.
    PossibleDynamicLoad,
}

/// How a fact's specifier resolved.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Found among the scanned files; `targets` lists them.
    Resolved,
    /// A package from outside the repository (npm, PyPI, the standard
    /// library, ...).
    External,
    /// Points into the repository but matched no file.
    Unresolved,
    /// Could be one of several files. Reserved: no rule produces it yet.
    Ambiguous,
    /// The specifier cannot be resolved statically, e.g. a template literal.
    Unsupported,
}

/// How sure the analysis is that the dependency exists.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// An import statement or call written in the source.
    Direct,
    /// A declaration that loads a file: Rust `mod name;`.
    Declaration,
    /// Inferred from a call, a string or a framework convention.
    Inferred,
    /// May happen; the target is not known statically.
    Possibility,
}

/// Where a fact was seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Evidence {
    /// The file the fact was read from.
    pub file: PathBuf,
    /// Line number, from 1. Absent for facts from a convention rather than
    /// a line, such as Django app modules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Byte range `[start, end)` of the specifier in the file, when cheap
    /// to know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<[usize; 2]>,
    /// Id of the rule that produced the fact, e.g. `js.import`,
    /// `python.from_import`, `rust.mod`.
    pub rule: String,
}

/// One dependency of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DependencyFact {
    /// The file that depends on something.
    pub source: PathBuf,
    /// The dependency as written: an import path, a module name, a file
    /// name, or the target's own path for convention facts.
    pub specifier: String,
    /// Files the specifier resolved to; empty unless `resolution` is
    /// `resolved`. A Python import can load several (its packages'
    /// `__init__.py` files and the module).
    pub targets: Vec<PathBuf>,
    pub kind: DependencyKind,
    pub resolution: Resolution,
    pub certainty: Certainty,
    /// When the dependency applies, if not always: `dynamic` for
    /// `import()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub evidence: Evidence,
}

/// Every dependency fact in a report.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DependencyFacts {
    /// See [`RULES_VERSION`]. 0 in reports from before facts.
    #[serde(default)]
    pub rules_version: u32,
    /// Sorted by source, line, span, specifier and kind.
    #[serde(default)]
    pub facts: Vec<DependencyFact>,
}

impl DependencyFact {
    /// A fact seen on `line` (from 1) of `source`.
    pub(crate) fn new(
        source: &Path,
        specifier: &str,
        kind: DependencyKind,
        certainty: Certainty,
        rule: &str,
    ) -> Self {
        DependencyFact {
            source: source.to_path_buf(),
            specifier: specifier.to_string(),
            targets: Vec::new(),
            kind,
            resolution: Resolution::Unresolved,
            certainty,
            condition: None,
            evidence: Evidence {
                file: source.to_path_buf(),
                line: None,
                span: None,
                rule: rule.to_string(),
            },
        }
    }

    pub(crate) fn line(mut self, line: usize) -> Self {
        self.evidence.line = Some(line);
        self
    }

    /// Record where `specifier` sits in `content`, when it is a slice of it.
    pub(crate) fn span_in(mut self, content: &str, specifier: &str) -> Self {
        self.evidence.span = span_of(content, specifier);
        self
    }

    pub(crate) fn condition(mut self, condition: &str) -> Self {
        self.condition = Some(condition.to_string());
        self
    }

    /// Resolved to `targets`.
    pub(crate) fn resolved(mut self, targets: Vec<PathBuf>) -> Self {
        self.resolution = Resolution::Resolved;
        self.targets = targets;
        self
    }

    pub(crate) fn resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = resolution;
        self
    }

    /// The same fact with every path passed through `f`.
    pub fn map_paths(self, f: impl Fn(&Path) -> PathBuf) -> Self {
        DependencyFact {
            source: f(&self.source),
            targets: self.targets.iter().map(|t| f(t)).collect(),
            evidence: Evidence {
                file: f(&self.evidence.file),
                ..self.evidence
            },
            ..self
        }
    }
}

/// Byte range of `part` within `whole`, when `part` is a slice of it.
fn span_of(whole: &str, part: &str) -> Option<[usize; 2]> {
    let start = (part.as_ptr() as usize).checked_sub(whole.as_ptr() as usize)?;
    let end = start + part.len();
    (end <= whole.len()).then_some([start, end])
}

/// Sort facts into report order.
pub fn sort(facts: &mut [DependencyFact]) {
    facts.sort_by(|a, b| {
        (
            &a.source,
            a.evidence.line,
            a.evidence.span,
            &a.specifier,
            a.kind,
            &a.evidence.rule,
            &a.targets,
        )
            .cmp(&(
                &b.source,
                b.evidence.line,
                b.evidence.span,
                &b.specifier,
                b.kind,
                &b.evidence.rule,
                &b.targets,
            ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_only_for_slices_of_the_content() {
        let content = "import a from './a';";
        let spec = &content[15..18];
        assert_eq!(span_of(content, spec), Some([15, 18]));
        assert_eq!(span_of(content, "./a"), None);
    }

    #[test]
    fn sorts_by_source_then_line() {
        let fact = |source: &str, line: usize| {
            DependencyFact::new(
                Path::new(source),
                "x",
                DependencyKind::RuntimeImport,
                Certainty::Direct,
                "js.import",
            )
            .line(line)
        };
        let mut facts = vec![fact("b.ts", 1), fact("a.ts", 9), fact("a.ts", 2)];
        sort(&mut facts);
        let order: Vec<(String, Option<usize>)> = facts
            .iter()
            .map(|f| (f.source.display().to_string(), f.evidence.line))
            .collect();
        assert_eq!(
            order,
            [
                ("a.ts".to_string(), Some(2)),
                ("a.ts".to_string(), Some(9)),
                ("b.ts".to_string(), Some(1)),
            ]
        );
    }

    #[test]
    fn kinds_serialize_in_snake_case() {
        assert_eq!(
            serde_json::to_value(DependencyKind::PossibleDynamicLoad).unwrap(),
            "possible_dynamic_load"
        );
        assert_eq!(serde_json::to_value(Certainty::Direct).unwrap(), "direct");
    }
}
