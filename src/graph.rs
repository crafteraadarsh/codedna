//! The plain dependency graph, built from dependency facts.
//!
//! `file → [files it depends on]` holds the targets of every resolved fact
//! (and every candidate of an ambiguous one, since any of them may be the
//! file loaded),
//! in the order the facts were found, each target once. Unresolved,
//! external and unsupported facts add no edge, and neither do possibilities:
//! a resolved target that only might be loaded (a shadowed `require`, a
//! text-scan fallback) is reported as a fact but never drawn as an edge. Every source file is a node,
//! even with no dependencies.

use crate::dependency_graph::DependencyGraph;
use crate::facts::{Certainty, DependencyFact, DependencyKind, Resolution};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Build the graph with a node for each of `nodes` and an edge for each
/// target of a resolved or ambiguous fact in `facts` that is more than a possibility.
pub fn from_facts<'a>(
    nodes: impl IntoIterator<Item = PathBuf>,
    facts: impl IntoIterator<Item = &'a DependencyFact>,
) -> DependencyGraph {
    let mut graph: DependencyGraph = nodes.into_iter().map(|n| (n, Vec::new())).collect();
    for fact in facts {
        if !is_drawn(fact) {
            continue;
        }
        let deps = graph.entry(fact.source.clone()).or_default();
        for target in &fact.targets {
            if !deps.contains(target) {
                deps.push(target.clone());
            }
        }
    }
    graph
}

/// What an edge of the dependency graph means for the file it points at.
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
pub enum EdgeKind {
    /// Loaded when the source runs: an import, re-export, worker, process
    /// or URL reference that resolved to this file.
    Runtime,
    /// One of several files that may be loaded: a candidate of an
    /// ambiguous import, or a match of a lazy pattern.
    Possible,
    /// Types only, erased at runtime.
    Type,
}

impl EdgeKind {
    /// The kind of the edges a drawn fact adds.
    pub fn of(fact: &DependencyFact) -> EdgeKind {
        if fact.kind == DependencyKind::TypeOnlyImport {
            EdgeKind::Type
        } else if fact.resolution == Resolution::Ambiguous
            || fact.kind == DependencyKind::PossibleDynamicLoad
        {
            EdgeKind::Possible
        } else {
            EdgeKind::Runtime
        }
    }
}

/// Whether `fact` draws edges in the graph.
pub fn is_drawn(fact: &DependencyFact) -> bool {
    matches!(
        fact.resolution,
        Resolution::Resolved | Resolution::Ambiguous
    ) && fact.certainty != Certainty::Possibility
}

/// Typed edges: for each source, the files it depends on and the strongest
/// kind of edge to each (runtime over possible over type-only), in the
/// order the facts were found. The same edges as [`from_facts`].
pub type TypedGraph = HashMap<PathBuf, Vec<(PathBuf, EdgeKind)>>;

/// The typed graph with a node for each of `nodes` and the typed edges
/// of the drawn facts in `facts`.
pub fn typed_edges<'a>(
    nodes: impl IntoIterator<Item = PathBuf>,
    facts: impl IntoIterator<Item = &'a DependencyFact>,
) -> TypedGraph {
    let mut graph: TypedGraph = nodes.into_iter().map(|n| (n, Vec::new())).collect();
    for fact in facts {
        if !is_drawn(fact) {
            continue;
        }
        let kind = EdgeKind::of(fact);
        let deps = graph.entry(fact.source.clone()).or_default();
        for target in &fact.targets {
            match deps.iter_mut().find(|(t, _)| t == target) {
                Some((_, k)) => *k = (*k).min(kind),
                None => deps.push((target.clone(), kind)),
            }
        }
    }
    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::DependencyKind;
    use std::path::Path;

    fn fact(source: &str, target: &str, resolution: Resolution) -> DependencyFact {
        let f = DependencyFact::new(
            Path::new(source),
            target,
            DependencyKind::RuntimeImport,
            Certainty::Direct,
            "js.import",
        );
        match resolution {
            Resolution::Resolved => f.resolved(vec![PathBuf::from(target)]),
            Resolution::Ambiguous => {
                let mut f = f.resolution(Resolution::Ambiguous);
                f.targets = vec![PathBuf::from(target)];
                f
            }
            other => f.resolution(other),
        }
    }

    #[test]
    fn edges_come_from_resolved_and_ambiguous_facts_in_order_once() {
        let facts = [
            fact("a", "c", Resolution::Resolved),
            fact("a", "x", Resolution::Unresolved),
            fact("a", "b", Resolution::Resolved),
            fact("a", "c", Resolution::Resolved),
            fact("a", "react", Resolution::External),
            fact("a", "d", Resolution::Ambiguous),
        ];
        let graph = from_facts([PathBuf::from("a"), PathBuf::from("lonely")], &facts);
        assert_eq!(
            graph[Path::new("a")],
            [PathBuf::from("c"), PathBuf::from("b"), PathBuf::from("d")]
        );
        assert!(graph[Path::new("lonely")].is_empty());
        assert_eq!(graph.len(), 2);
    }

    #[test]
    fn typed_edges_keep_the_strongest_kind_of_the_same_edges() {
        let mut type_only = fact("a", "b", Resolution::Resolved);
        type_only.kind = DependencyKind::TypeOnlyImport;
        let mut shadowed = fact("a", "e", Resolution::Resolved);
        shadowed.certainty = Certainty::Possibility;
        let facts = [
            type_only.clone(),
            fact("a", "c", Resolution::Resolved),
            fact("a", "d", Resolution::Ambiguous),
            type_only,
            fact("a", "c", Resolution::Resolved),
            shadowed,
        ];
        let mut runtime_too = fact("a", "d", Resolution::Resolved);
        runtime_too.source = PathBuf::from("z");
        let typed = typed_edges([PathBuf::from("lonely")], facts.iter().chain([&runtime_too]));
        assert_eq!(
            typed[Path::new("a")],
            [
                (PathBuf::from("b"), EdgeKind::Type),
                (PathBuf::from("c"), EdgeKind::Runtime),
                (PathBuf::from("d"), EdgeKind::Possible),
            ]
        );
        // The same edges as the plain graph, and the same nodes.
        let plain = from_facts([PathBuf::from("lonely")], facts.iter().chain([&runtime_too]));
        for (source, deps) in &plain {
            let typed: Vec<&PathBuf> = typed[source].iter().map(|(t, _)| t).collect();
            assert_eq!(typed, deps.iter().collect::<Vec<_>>());
        }
        assert_eq!(typed.len(), plain.len());
        // A runtime edge beats a type-only one to the same file.
        let mut both = facts[0].clone();
        both.kind = DependencyKind::RuntimeImport;
        let typed = typed_edges([], [&facts[0], &both]);
        assert_eq!(typed[Path::new("a")], [(PathBuf::from("b"), EdgeKind::Runtime)]);
    }
}
