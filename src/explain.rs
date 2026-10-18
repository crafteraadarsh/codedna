//! Why CodeDna says what it says: `codedna explain` and `codedna query`.
//!
//! Every answer is read from the analysis itself, never inferred: a file's
//! usage comes with the entry point that reaches it, why that file is an
//! entry point, and the dependency fact (file, line, rule) behind each step
//! of the path; a finding comes with its evidence and the conditions that
//! could change it; and every answer lists the assumptions it rests on and
//! the provenance of the analysis (analyzer, schema and rules versions,
//! adapters, repository revision and a content snapshot hash).
//!
//! Stable ids name what can be explained: `file:<path>`, `dead:<path>` for a
//! dead-code finding and `cycle:<path>` for a cycle, named after its first
//! file in path order (a file belongs to at most one cycle group).

use crate::analysis::{dotted, AnalysisResult};
use crate::dead_code_detector::{
    Classification, DeadCodeCandidate, DeadStatus, Role, RootSource, UsageKind,
};
use crate::dependency_graph::normalize;
use crate::facts::{Certainty, DependencyFact, DependencyKind, Resolution};
use crate::graph::{self, EdgeKind, TypedGraph};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Facts listed per answer unless a limit is given.
pub const DEFAULT_LIMIT: usize = 50;

/// The most facts any answer lists.
pub const MAX_LIMIT: usize = 1000;

/// Where the analysis came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Provenance {
    pub analyzer: crate::analysis::Analyzer,
    pub schema_version: u32,
    /// Version of the dependency-fact rules.
    pub rules_version: u32,
    /// Always `static`: files were read, never run.
    pub analysis: &'static str,
    /// Active framework adapters as `adapter@version (manifest)`.
    pub adapters: Vec<String>,
    /// The Git commit checked out, if the folder is in a Git repository.
    /// Uncommitted changes are not part of it; `snapshot` covers them.
    pub revision: Option<String>,
    /// 64-bit hash of every analyzed file's path and content, in hex: the
    /// same code gives the same snapshot.
    pub snapshot: String,
}

/// The analysis with what `explain` and `query` need to answer from it.
pub struct Explainer<'a> {
    result: &'a AnalysisResult,
    class: Classification,
    typed: TypedGraph,
    /// Report path for each `/`-separated path without `./`.
    files: HashMap<String, PathBuf>,
    by_source: HashMap<String, Vec<&'a DependencyFact>>,
    by_target: HashMap<String, Vec<&'a DependencyFact>>,
    provenance: Provenance,
}

/// Why a file is an entry point.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RootInfo {
    pub file: PathBuf,
    /// `production`, `types`, `test` or `tooling`.
    pub role: &'static str,
    pub reason: String,
    /// The manifest line that names it, for a configured entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::facts::Evidence>,
}

/// One edge of a path, with the fact behind it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub from: PathBuf,
    pub to: PathBuf,
    /// `runtime`, `possible` or `type`: how the edge counts for usage.
    pub edge: &'static str,
    pub kind: DependencyKind,
    pub specifier: String,
    pub certainty: Certainty,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub evidence: crate::facts::Evidence,
}

/// Facts, at most a limit of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactList {
    pub total: usize,
    pub truncated: bool,
    pub facts: Vec<DependencyFact>,
}

/// A dead-code finding with its stable id.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeadFinding {
    pub id: String,
    #[serde(flatten)]
    pub candidate: DeadCodeCandidate,
}

/// Why a file is used, or not.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileExplanation {
    /// `file:<path>`, or the finding id it was asked for by.
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub file: PathBuf,
    pub language: Option<String>,
    pub loc: Option<usize>,
    /// Whether CodeDna followed this file's imports. When not, nothing is
    /// said about its usage.
    pub analyzed: bool,
    pub usage: Option<UsageKind>,
    pub summary: String,
    /// The entry point the witness path starts from.
    pub root: Option<RootInfo>,
    /// The first path found from an entry point to the file; empty for an
    /// entry point itself and for an unreachable file.
    pub witness: Vec<Step>,
    pub dead_code: Option<DeadFinding>,
    /// The id of the cycle the file is in.
    pub cycle: Option<String>,
    pub imported_by: FactList,
    pub imports: FactList,
    pub assumptions: Vec<String>,
    pub provenance: Provenance,
}

/// A cycle and the edge that closes each step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CycleExplanation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind_of: &'static str,
    pub kind: crate::cycles::CycleKind,
    pub files: Vec<PathBuf>,
    pub edges: Vec<Step>,
    pub assumptions: Vec<String>,
    pub provenance: Provenance,
}

/// What `explain` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Explanation {
    File(Box<FileExplanation>),
    Cycle(Box<CycleExplanation>),
}

/// The answer to `query path`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathAnswer {
    pub op: &'static str,
    pub from: PathBuf,
    pub to: PathBuf,
    pub found: bool,
    /// The shortest chain of drawn dependencies, through any kind of edge.
    pub steps: Vec<Step>,
    pub provenance: Provenance,
}

/// The answer to `query references`, `imports` or `unresolved`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactsAnswer {
    pub op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub total: usize,
    pub truncated: bool,
    pub facts: Vec<DependencyFact>,
    /// For `references`: the manifests that name the file as an entry.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub configured_by: Vec<DependencyFact>,
    pub provenance: Provenance,
}

/// The answer to `query entrypoints`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntrypointsAnswer {
    pub op: &'static str,
    /// Entry points per role.
    pub counts: BTreeMap<&'static str, usize>,
    pub total: usize,
    pub truncated: bool,
    pub entrypoints: Vec<RootInfo>,
    pub assumptions: Vec<String>,
    pub provenance: Provenance,
}

/// A path as a lookup key: `/`-separated, without `./`.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut s = text.as_str();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest;
    }
    s.to_string()
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Production => "production",
        Role::Types => "types",
        Role::Test => "test",
        Role::Tooling => "tooling",
    }
}

fn edge_name(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Runtime => "runtime",
        EdgeKind::Possible => "possible",
        EdgeKind::Type => "type",
    }
}

/// The checked-out commit of the Git repository holding `root`, if any.
fn revision(root: &Path) -> Option<String> {
    let repo = git2::Repository::discover(root).ok()?;
    let commit = repo.head().ok()?.peel_to_commit().ok()?;
    Some(commit.id().to_string())
}

/// Hash of every file's path and content, in path order.
fn snapshot(root: &Path, result: &AnalysisResult) -> String {
    let mut files: Vec<&PathBuf> = result.file_breakdown.iter().map(|f| &f.file).collect();
    files.sort_unstable();
    let hashes: Vec<u64> = files
        .par_iter()
        .map(|file| {
            let content = std::fs::read(root.join(file)).unwrap_or_default();
            let mut buf = key(file).into_bytes();
            buf.push(0);
            buf.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(&content).to_le_bytes());
            xxhash_rust::xxh3::xxh3_64(&buf)
        })
        .collect();
    let bytes: Vec<u8> = hashes.iter().flat_map(|h| h.to_le_bytes()).collect();
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&bytes))
}

/// Provenance of `result`, analyzed from `root`.
pub fn provenance(root: &Path, result: &AnalysisResult) -> Provenance {
    let mut adapters: Vec<String> = result
        .dependencies
        .adapters
        .iter()
        .map(|a| {
            format!(
                "{}@{} ({})",
                a.adapter,
                a.version,
                key(&a.manifest).replace('\\', "/")
            )
        })
        .collect();
    adapters.sort();
    adapters.dedup();
    Provenance {
        analyzer: result.analyzer.clone(),
        schema_version: result.schema_version,
        rules_version: result.dependencies.rules_version,
        analysis: "static",
        adapters,
        revision: revision(root),
        snapshot: snapshot(root, result),
    }
}

impl<'a> Explainer<'a> {
    /// Answers about `result`, analyzed from `root`, whose files were
    /// classified as `class` (see [`crate::analysis::analyze_detailed`]).
    pub fn new(root: &Path, result: &'a AnalysisResult, class: Classification) -> Self {
        let typed = graph::typed_edges(
            result.dependency_graph.keys().cloned(),
            &result.dependencies.facts,
        );
        let mut files: HashMap<String, PathBuf> = HashMap::new();
        for f in &result.file_breakdown {
            files.insert(key(&f.file), f.file.clone());
        }
        for f in result.dependency_graph.keys() {
            files.entry(key(f)).or_insert_with(|| f.clone());
        }
        let mut by_source: HashMap<String, Vec<&DependencyFact>> = HashMap::new();
        let mut by_target: HashMap<String, Vec<&DependencyFact>> = HashMap::new();
        for fact in &result.dependencies.facts {
            by_source.entry(key(&fact.source)).or_default().push(fact);
            let mut seen = HashSet::new();
            for target in &fact.targets {
                if seen.insert(key(target)) {
                    by_target.entry(key(target)).or_default().push(fact);
                }
            }
        }
        Explainer {
            result,
            class,
            typed,
            files,
            by_source,
            by_target,
            provenance: provenance(root, result),
        }
    }

    /// The report path of a file given as typed (`src/a.ts`, `./src/a.ts`,
    /// `file:src/a.ts`).
    pub fn file(&self, input: &str) -> Result<PathBuf, String> {
        let bare = input.strip_prefix("file:").unwrap_or(input);
        let wanted = key(&normalize(Path::new(&bare.replace('\\', "/"))));
        self.files.get(&wanted).cloned().ok_or_else(|| {
            format!(
                "No file '{}' in the analysis. Give a path relative to the analyzed folder.",
                crate::safe_text::sanitize(&dotted(&wanted))
            )
        })
    }

    /// Explain a file or a finding id (`file:`, `dead:`, `cycle:`).
    pub fn explain(&self, target: &str) -> Result<Explanation, String> {
        if let Some(rest) = target.strip_prefix("cycle:") {
            return self
                .explain_cycle(rest)
                .map(|c| Explanation::Cycle(Box::new(c)));
        }
        let (id_prefix, rest) = match target.strip_prefix("dead:") {
            Some(rest) => ("dead", rest),
            None => ("file", target.strip_prefix("file:").unwrap_or(target)),
        };
        let file = self.file(rest)?;
        let mut explained = self.explain_file(&file);
        if id_prefix == "dead" {
            match &explained.dead_code {
                Some(finding) => explained.id = finding.id.clone(),
                None => {
                    return Err(format!(
                        "'{}' is not a dead-code candidate: it is {}.",
                        crate::safe_text::sanitize(&dotted(&key(&file))),
                        explained.usage.map_or("not analyzed", |u| u.label())
                    ))
                }
            }
        }
        Ok(Explanation::File(Box::new(explained)))
    }

    fn root_info(&self, file: &Path) -> Option<RootInfo> {
        let (role, source) = self.class.roots.get(file)?;
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (reason, evidence) = match source {
            RootSource::Name if *role == Role::Tooling => {
                (format!("named like a tool config entry point: {name}"), None)
            }
            RootSource::Name => (format!("named like an entry point: {name}"), None),
            RootSource::Convention => (
                match role {
                    Role::Test => "loaded by a test runner by convention (test, setup or mock file)",
                    Role::Types => "read by the type checker by convention (declaration file)",
                    Role::Tooling => {
                        "loaded by build or development tooling by convention (config, story, task runner)"
                    }
                    Role::Production => {
                        "loaded by its framework by convention (route, component, migration or build target)"
                    }
                }
                .to_string(),
                None,
            ),
            RootSource::Configured(path) => {
                let wanted = key(path);
                let fact = self
                    .result
                    .dependencies
                    .entries
                    .iter()
                    .find(|f| f.targets.iter().any(|t| key(&normalize(t)) == wanted));
                let folder = if key(file) == wanted {
                    String::new()
                } else {
                    format!(" (through the folder {})", dotted(&wanted))
                };
                match fact {
                    Some(f) => (
                        format!(
                            "configured entry: {} in {}{folder}",
                            f.evidence.rule,
                            dotted(&key(&f.source))
                        ),
                        Some(f.evidence.clone()),
                    ),
                    None => (format!("configured entry{folder}"), None),
                }
            }
        };
        Some(RootInfo {
            file: file.to_path_buf(),
            role: role_name(*role),
            reason,
            evidence,
        })
    }

    /// The fact behind the edge `from → to`, of the kind the graph gives it.
    fn step(&self, from: &Path, to: &Path) -> Option<Step> {
        let kind = self
            .typed
            .get(from)?
            .iter()
            .find(|(t, _)| t == to)
            .map(|(_, k)| *k)?;
        let wanted = key(to);
        let fact = self
            .by_source
            .get(&key(from))?
            .iter()
            .copied()
            .filter(|f| graph::is_drawn(f) && EdgeKind::of(f) == kind)
            .find(|f| f.targets.iter().any(|t| key(t) == wanted))?;
        Some(Step {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            edge: edge_name(kind),
            kind: fact.kind,
            specifier: fact.specifier.clone(),
            certainty: fact.certainty,
            condition: fact.condition.clone(),
            evidence: fact.evidence.clone(),
        })
    }

    fn steps(&self, path: &[PathBuf]) -> Vec<Step> {
        path.windows(2)
            .filter_map(|pair| self.step(&pair[0], &pair[1]))
            .collect()
    }

    fn list(facts: Vec<&DependencyFact>, limit: usize) -> FactList {
        let total = facts.len();
        FactList {
            total,
            truncated: total > limit,
            facts: facts.into_iter().take(limit).cloned().collect(),
        }
    }

    fn language(&self, file: &Path) -> (Option<String>, Option<usize>) {
        self.result
            .file_breakdown
            .iter()
            .find(|f| f.file == file)
            .map_or((None, None), |f| (Some(f.language.clone()), Some(f.loc)))
    }

    /// Facts that resolve to `file`.
    fn references(&self, file: &Path) -> Vec<&'a DependencyFact> {
        self.by_target.get(&key(file)).cloned().unwrap_or_default()
    }

    fn imports(&self, file: &Path) -> Vec<&'a DependencyFact> {
        self.by_source.get(&key(file)).cloned().unwrap_or_default()
    }

    fn cycle_of(&self, file: &Path) -> Option<String> {
        self.result
            .cycle_details
            .iter()
            .find(|c| c.files.iter().any(|f| f == file))
            .map(|c| cycle_id(&c.files))
    }

    fn explain_file(&self, file: &Path) -> FileExplanation {
        let (language, loc) = self.language(file);
        let analyzed = self.result.dependency_graph.contains_key(file);
        let usage = analyzed.then(|| {
            self.class
                .usage
                .get(file)
                .copied()
                .unwrap_or(UsageKind::Unreachable)
        });
        let reached = usage.is_some_and(|u| u != UsageKind::Unreachable);
        let path = if reached {
            self.class.path(file)
        } else {
            Vec::new()
        };
        let root = path.first().and_then(|r| self.root_info(r));
        let witness = self.steps(&path);
        let dead_code = self
            .result
            .dead_code_candidates()
            .into_iter()
            .find(|c| c.file == file)
            .map(|candidate| DeadFinding {
                id: format!("dead:{}", dotted(&key(file))),
                candidate,
            });
        let summary = match (usage, &root) {
            (None, _) => format!(
                "Not analyzed: CodeDna does not follow imports in {} files, so it says nothing about whether this file is used.",
                language.as_deref().unwrap_or("these")
            ),
            (Some(UsageKind::Unreachable), _) => {
                let status = dead_code
                    .as_ref()
                    .map_or(DeadStatus::Unreachable, |d| d.candidate.status);
                format!(
                    "Unreachable: no analyzed import reaches it from an entry point (status: {}).",
                    status.as_str()
                )
            }
            (Some(u), Some(r)) if witness.is_empty() => {
                format!("{}: it is an entry point ({}).", capital(u.label()), r.reason)
            }
            (Some(u), Some(r)) => format!(
                "{}: reached from {} ({}) in {} step{}.",
                capital(u.label()),
                dotted(&key(&r.file)),
                r.reason,
                witness.len(),
                if witness.len() == 1 { "" } else { "s" }
            ),
            (Some(u), None) => format!("{}.", capital(u.label())),
        };
        let mut imports = self.imports(file);
        imports.sort_by_key(|f| (f.evidence.line, f.evidence.span));
        let assumptions = self.file_assumptions(file, analyzed, usage, language.as_deref());
        FileExplanation {
            id: format!("file:{}", dotted(&key(file))),
            kind: "file",
            file: file.to_path_buf(),
            language,
            loc,
            analyzed,
            usage,
            summary,
            root,
            witness,
            dead_code,
            cycle: self.cycle_of(file),
            imported_by: Self::list(self.references(file), DEFAULT_LIMIT),
            imports: Self::list(imports, DEFAULT_LIMIT),
            assumptions,
            provenance: self.provenance.clone(),
        }
    }

    /// What every graph answer rests on.
    fn common_assumptions(&self) -> Vec<String> {
        let mut out = vec![
            "Static analysis: files were read, never run. Only dependencies CodeDna reads are \
             edges; code loaded by a name built at runtime (reflection, plugin loaders, computed \
             paths) is not followed."
                .to_string(),
        ];
        let adapters = if self.provenance.adapters.is_empty() {
            "none active".to_string()
        } else {
            self.provenance.adapters.join(", ")
        };
        out.push(format!(
            "Entry points are files named like entry points, files loaded by convention (tests, \
             configs, routes, declaration files) and files framework adapters name (adapters: \
             {adapters})."
        ));
        let coverage = &self.result.coverage;
        if coverage.is_partial() {
            out.push(format!(
                "Coverage is partial: {}. Files reached only through what was not followed look \
                 less used than they are.",
                coverage.summary.trim_end_matches('.')
            ));
        }
        out
    }

    fn file_assumptions(
        &self,
        file: &Path,
        analyzed: bool,
        usage: Option<UsageKind>,
        language: Option<&str>,
    ) -> Vec<String> {
        let mut out = Vec::new();
        if !analyzed {
            out.push(format!(
                "{} imports are not followed: this file is not in the dependency graph, so it is \
                 neither confirmed nor a dead-code candidate.",
                language.unwrap_or("This file's")
            ));
        }
        out.extend(self.common_assumptions());
        let shown = key(file);
        if let Some(failure) = self
            .result
            .coverage
            .parse
            .failures
            .iter()
            .find(|f| key(Path::new(&f.file)) == shown)
        {
            out.push(format!(
                "This file did not fully parse ({}, line {}): its dependencies come from what \
                 could be recovered.",
                failure.status, failure.line
            ));
        }
        let dynamic = self
            .imports(file)
            .iter()
            .filter(|f| f.certainty == Certainty::Possibility && !graph::is_drawn(f))
            .count();
        if dynamic > 0 {
            out.push(format!(
                "This file has {dynamic} dynamic load{} whose target is not known statically; \
                 they add no edges.",
                if dynamic == 1 { "" } else { "s" }
            ));
        }
        match usage {
            Some(UsageKind::Unreachable) => out.push(
                "Unreachable is not proof that the file is unused: see the finding's status, \
                 confidence and conditions, and check before deleting."
                    .to_string(),
            ),
            Some(_) => out.push(
                "The witness path is the first one found, breadth-first from the entry points, \
                 through the strongest kind of edge available; other paths may exist."
                    .to_string(),
            ),
            None => {}
        }
        out
    }

    fn explain_cycle(&self, name: &str) -> Result<CycleExplanation, String> {
        let wanted = key(&normalize(Path::new(&name.replace('\\', "/"))));
        let cycle = self
            .result
            .cycle_details
            .iter()
            .find(|c| key(&first_file(&c.files)) == wanted)
            .ok_or_else(|| {
                format!(
                    "No cycle 'cycle:{}'. A cycle id names the cycle's first file in path order; \
                     `codedna explain <file>` gives the id of the cycle a file is in.",
                    crate::safe_text::sanitize(&dotted(&wanted))
                )
            })?;
        let mut closed = cycle.files.clone();
        if let Some(first) = cycle.files.first() {
            closed.push(first.clone());
        }
        let mut assumptions = self.common_assumptions();
        assumptions.push(
            "The cycle shown is the shortest loop of its kind in a group of files that import \
             each other; the group may hold other loops."
                .to_string(),
        );
        Ok(CycleExplanation {
            id: cycle_id(&cycle.files),
            kind_of: "cycle",
            kind: cycle.kind,
            files: cycle.files.clone(),
            edges: self.steps(&closed),
            assumptions,
            provenance: self.provenance.clone(),
        })
    }

    /// `query references <file>`.
    pub fn query_references(&self, input: &str, limit: usize) -> Result<FactsAnswer, String> {
        let file = self.file(input)?;
        let facts = self.references(&file);
        let wanted = key(&file);
        let configured_by = self
            .result
            .dependencies
            .entries
            .iter()
            .filter(|f| f.targets.iter().any(|t| key(t) == wanted))
            .cloned()
            .collect();
        Ok(self.facts_answer("references", Some(file), facts, configured_by, limit))
    }

    /// `query imports <file>`.
    pub fn query_imports(&self, input: &str, limit: usize) -> Result<FactsAnswer, String> {
        let file = self.file(input)?;
        let mut facts = self.imports(&file);
        facts.sort_by_key(|f| (f.evidence.line, f.evidence.span));
        Ok(self.facts_answer("imports", Some(file), facts, Vec::new(), limit))
    }

    /// `query unresolved [--file]`: dependencies that matched no file, or
    /// that CodeDna cannot resolve (`unresolved`, `unsupported`).
    pub fn query_unresolved(
        &self,
        input: Option<&str>,
        limit: usize,
    ) -> Result<FactsAnswer, String> {
        let file = input.map(|i| self.file(i)).transpose()?;
        let facts: Vec<&DependencyFact> = match &file {
            Some(f) => self.imports(f),
            None => self.result.dependencies.facts.iter().collect(),
        }
        .into_iter()
        .filter(|f| {
            matches!(
                f.resolution,
                Resolution::Unresolved | Resolution::Unsupported
            )
        })
        .collect();
        Ok(self.facts_answer("unresolved", file, facts, Vec::new(), limit))
    }

    fn facts_answer(
        &self,
        op: &'static str,
        file: Option<PathBuf>,
        facts: Vec<&DependencyFact>,
        configured_by: Vec<DependencyFact>,
        limit: usize,
    ) -> FactsAnswer {
        let list = Self::list(facts, limit);
        FactsAnswer {
            op,
            file,
            total: list.total,
            truncated: list.truncated,
            facts: list.facts,
            configured_by,
            provenance: self.provenance.clone(),
        }
    }

    /// `query path <from> <to>`: the shortest chain of drawn dependencies.
    pub fn query_path(&self, from: &str, to: &str) -> Result<PathAnswer, String> {
        let from = self.file(from)?;
        let to = self.file(to)?;
        let mut parent: HashMap<&PathBuf, &PathBuf> = HashMap::new();
        let mut seen: HashSet<&PathBuf> = HashSet::new();
        let mut queue: VecDeque<&PathBuf> = VecDeque::new();
        let mut found = false;
        if let Some((start, _)) = self.typed.get_key_value(&from) {
            seen.insert(start);
            queue.push_back(start);
        }
        while let Some(at) = queue.pop_front() {
            if *at == to {
                found = true;
                break;
            }
            let mut next: Vec<&PathBuf> = self
                .typed
                .get(at)
                .into_iter()
                .flatten()
                .map(|(t, _)| t)
                .collect();
            next.sort_unstable();
            for n in next {
                if seen.insert(n) {
                    parent.insert(n, at);
                    queue.push_back(n);
                }
            }
        }
        let mut path = Vec::new();
        if found {
            let mut at = &to;
            path.push(to.clone());
            while let Some(p) = parent.get(at) {
                path.push((*p).clone());
                at = p;
            }
            path.reverse();
        }
        Ok(PathAnswer {
            op: "path",
            steps: self.steps(&path),
            from,
            to,
            found,
            provenance: self.provenance.clone(),
        })
    }

    /// `query entrypoints`: every entry point, by role, with why.
    pub fn query_entrypoints(&self, limit: usize) -> EntrypointsAnswer {
        let mut roots: Vec<(&Role, &PathBuf)> = self
            .class
            .roots
            .iter()
            .map(|(f, (role, _))| (role, f))
            .collect();
        roots.sort();
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (role, _) in &roots {
            *counts.entry(role_name(**role)).or_default() += 1;
        }
        let total = roots.len();
        EntrypointsAnswer {
            op: "entrypoints",
            counts,
            total,
            truncated: total > limit,
            entrypoints: roots
                .into_iter()
                .take(limit)
                .filter_map(|(_, f)| self.root_info(f))
                .collect(),
            assumptions: self.common_assumptions(),
            provenance: self.provenance.clone(),
        }
    }
}

/// The first file of a cycle in path order.
fn first_file(files: &[PathBuf]) -> PathBuf {
    files.iter().min().cloned().unwrap_or_default()
}

/// `cycle:<first file>`.
pub fn cycle_id(files: &[PathBuf]) -> String {
    format!("cycle:{}", dotted(&key(&first_file(files))))
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

fn at(evidence: &crate::facts::Evidence) -> String {
    match evidence.line {
        Some(line) => format!("{}:{line}", dotted(&key(&evidence.file))),
        None => dotted(&key(&evidence.file)),
    }
}

fn step_line(step: &Step) -> String {
    format!(
        "  {}  '{}' → {}  ({}, {}, {})\n",
        at(&step.evidence),
        crate::safe_text::sanitize(&step.specifier),
        dotted(&key(&step.to)),
        step.edge,
        serde_json::to_value(step.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        step.evidence.rule
    )
}

fn fact_line(fact: &DependencyFact) -> String {
    let resolution = serde_json::to_value(fact.resolution)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let targets: Vec<String> = fact.targets.iter().map(|t| dotted(&key(t))).collect();
    let to = if targets.is_empty() {
        resolution
    } else {
        targets.join(", ")
    };
    format!(
        "  {}  '{}' → {}  ({})\n",
        at(&fact.evidence),
        crate::safe_text::sanitize(&fact.specifier),
        to,
        fact.evidence.rule
    )
}

fn provenance_text(p: &Provenance) -> String {
    format!(
        "Provenance: {} {}, schema {}, rules {}, {} analysis, snapshot {}{}\n",
        p.analyzer.name,
        p.analyzer.version,
        p.schema_version,
        p.rules_version,
        p.analysis,
        p.snapshot,
        p.revision
            .as_ref()
            .map(|r| format!(", revision {}", &r[..r.len().min(12)]))
            .unwrap_or_default()
    )
}

fn assumptions_text(list: &[String]) -> String {
    let mut out = String::from("\nAssumptions:\n");
    for a in list {
        out.push_str(&format!("  - {a}\n"));
    }
    out
}

fn facts_text(title: &str, list: &FactList) -> String {
    if list.total == 0 {
        return format!("\n{title}: none\n");
    }
    let mut out = format!("\n{title} ({}):\n", list.total);
    for f in &list.facts {
        out.push_str(&fact_line(f));
    }
    if list.truncated {
        out.push_str(&format!("  … {} more\n", list.total - list.facts.len()));
    }
    out
}

/// `explain` for people.
pub fn render_text(explanation: &Explanation) -> String {
    match explanation {
        Explanation::File(e) => {
            let mut out = format!("{}  [{}]\n{}\n", dotted(&key(&e.file)), e.id, e.summary);
            if let Some(root) = &e.root {
                out.push_str(&format!(
                    "\nEntry point: {} ({}): {}\n",
                    dotted(&key(&root.file)),
                    root.role,
                    root.reason
                ));
                if let Some(ev) = &root.evidence {
                    out.push_str(&format!("  named at {} ({})\n", at(ev), ev.rule));
                }
            }
            if !e.witness.is_empty() {
                out.push_str("\nPath:\n");
                for step in &e.witness {
                    out.push_str(&step_line(step));
                }
            }
            if let Some(d) = &e.dead_code {
                out.push_str(&format!(
                    "\nDead-code finding {}: {} confidence, status {}: {}\n",
                    d.id,
                    d.candidate.confidence.as_str(),
                    d.candidate.status.as_str(),
                    d.candidate.reason
                ));
                for c in &d.candidate.conditions {
                    out.push_str(&format!("  may be loaded: {c}\n"));
                }
                for ev in &d.candidate.evidence {
                    out.push_str(&format!("  evidence: {}:{}\n", ev.file, ev.line));
                }
            }
            if let Some(c) = &e.cycle {
                out.push_str(&format!("\nIn cycle {c} (codedna explain {c})\n"));
            }
            if e.analyzed {
                out.push_str(&facts_text("Imported by", &e.imported_by));
                out.push_str(&facts_text("Imports", &e.imports));
            }
            out.push_str(&assumptions_text(&e.assumptions));
            out.push('\n');
            out.push_str(&provenance_text(&e.provenance));
            out
        }
        Explanation::Cycle(c) => {
            let mut out = format!(
                "{}  {} cycle of {} files\n\nEdges:\n",
                c.id,
                c.kind.label(),
                c.files.len()
            );
            for step in &c.edges {
                out.push_str(&step_line(step));
            }
            out.push_str(&assumptions_text(&c.assumptions));
            out.push('\n');
            out.push_str(&provenance_text(&c.provenance));
            out
        }
    }
}

/// `query references|imports|unresolved` for people.
pub fn render_facts_text(answer: &FactsAnswer) -> String {
    let title = match (&answer.file, answer.op) {
        (Some(f), "references") => format!("Dependencies on {}", dotted(&key(f))),
        (Some(f), "imports") => format!("Dependencies of {}", dotted(&key(f))),
        (Some(f), _) => format!("Unresolved dependencies of {}", dotted(&key(f))),
        (None, _) => "Unresolved dependencies".to_string(),
    };
    let mut out = facts_text(
        &title,
        &FactList {
            total: answer.total,
            truncated: answer.truncated,
            facts: answer.facts.clone(),
        },
    )
    .trim_start()
    .to_string();
    if !answer.configured_by.is_empty() {
        out.push_str("\nNamed as an entry by:\n");
        for f in &answer.configured_by {
            out.push_str(&format!("  {}  ({})\n", at(&f.evidence), f.evidence.rule));
        }
    }
    out.push('\n');
    out.push_str(&provenance_text(&answer.provenance));
    out
}

/// `query path` for people.
pub fn render_path_text(answer: &PathAnswer) -> String {
    let mut out = if answer.found {
        format!(
            "{} reaches {} in {} step{}:\n",
            dotted(&key(&answer.from)),
            dotted(&key(&answer.to)),
            answer.steps.len(),
            if answer.steps.len() == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "No chain of analyzed dependencies leads from {} to {}.\n",
            dotted(&key(&answer.from)),
            dotted(&key(&answer.to))
        )
    };
    for step in &answer.steps {
        out.push_str(&step_line(step));
    }
    out.push('\n');
    out.push_str(&provenance_text(&answer.provenance));
    out
}

/// `query entrypoints` for people.
pub fn render_entrypoints_text(answer: &EntrypointsAnswer) -> String {
    let counts: Vec<String> = answer
        .counts
        .iter()
        .map(|(role, n)| format!("{n} {role}"))
        .collect();
    let mut out = format!("{} entry points: {}\n\n", answer.total, counts.join(", "));
    for e in &answer.entrypoints {
        out.push_str(&format!(
            "  {}  {}: {}\n",
            dotted(&key(&e.file)),
            e.role,
            e.reason
        ));
    }
    if answer.truncated {
        out.push_str(&format!(
            "  … {} more\n",
            answer.total - answer.entrypoints.len()
        ));
    }
    out.push_str(&assumptions_text(&answer.assumptions));
    out.push('\n');
    out.push_str(&provenance_text(&answer.provenance));
    out
}
