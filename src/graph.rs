//! The plain dependency graph, built from dependency facts.
//!
//! `file → [files it depends on]` holds the targets of every resolved fact,
//! in the order the facts were found, each target once. Unresolved,
//! external and unsupported facts add no edge, and neither do possibilities:
//! a resolved target that only might be loaded (a shadowed `require`, a
//! text-scan fallback) is reported as a fact but never drawn as an edge. Every source file is a node,
//! even with no dependencies.

use crate::dependency_graph::DependencyGraph;
use crate::facts::{Certainty, DependencyFact, Resolution};
use std::path::PathBuf;

/// Build the graph with a node for each of `nodes` and an edge for each
/// target of a resolved fact in `facts` that is more than a possibility.
pub fn from_facts<'a>(
    nodes: impl IntoIterator<Item = PathBuf>,
    facts: impl IntoIterator<Item = &'a DependencyFact>,
) -> DependencyGraph {
    let mut graph: DependencyGraph = nodes.into_iter().map(|n| (n, Vec::new())).collect();
    for fact in facts {
        if fact.resolution != Resolution::Resolved || fact.certainty == Certainty::Possibility {
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
            other => f.resolution(other),
        }
    }

    #[test]
    fn edges_come_from_resolved_facts_only_in_order_once() {
        let facts = [
            fact("a", "c", Resolution::Resolved),
            fact("a", "x", Resolution::Unresolved),
            fact("a", "b", Resolution::Resolved),
            fact("a", "c", Resolution::Resolved),
            fact("a", "react", Resolution::External),
        ];
        let graph = from_facts([PathBuf::from("a"), PathBuf::from("lonely")], &facts);
        assert_eq!(
            graph[Path::new("a")],
            [PathBuf::from("c"), PathBuf::from("b")]
        );
        assert!(graph[Path::new("lonely")].is_empty());
        assert_eq!(graph.len(), 2);
    }
}
