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
use crate::facts::{Certainty, DependencyFact, Resolution};
use std::path::PathBuf;

/// Build the graph with a node for each of `nodes` and an edge for each
/// target of a resolved or ambiguous fact in `facts` that is more than a possibility.
pub fn from_facts<'a>(
    nodes: impl IntoIterator<Item = PathBuf>,
    facts: impl IntoIterator<Item = &'a DependencyFact>,
) -> DependencyGraph {
    let mut graph: DependencyGraph = nodes.into_iter().map(|n| (n, Vec::new())).collect();
    for fact in facts {
        let drawn = matches!(
            fact.resolution,
            Resolution::Resolved | Resolution::Ambiguous
        );
        if !drawn || fact.certainty == Certainty::Possibility {
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
}
