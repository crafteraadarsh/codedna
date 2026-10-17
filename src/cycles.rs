//! Circular imports: groups of files that import each other.
//!
//! A cycle is found with Tarjan's strongly connected components on the
//! dependency graph. Every group of files that can reach each other through
//! imports is one finding, reported as the shortest cycle through its first
//! file:
//!
//! ```text
//! src/a.ts → src/b.ts → src/c.ts → src/a.ts
//! ```
//!
//! Only the imports the dependency graph tracks are seen. Type-only imports
//! count too, but each cycle says whether runtime imports alone close it, it
//! needs an import that may load one of several files, or it needs a
//! type-only import (erased when the code runs). A finding is something to
//! check, not proof of a runtime problem.

use crate::graph::{EdgeKind, TypedGraph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// Find circular imports in `graph`.
///
/// Returns one cycle per group of files that import each other, as the
/// files in import order starting from the group's first path; the last
/// file imports the first. A file that imports itself is a cycle of one.
/// Cycles are sorted shortest first, then by path, so output is stable.
pub fn find_cycles(graph: &HashMap<PathBuf, Vec<PathBuf>>) -> Vec<Vec<PathBuf>> {
    let typed: TypedGraph = graph
        .iter()
        .map(|(from, to)| {
            let to = to.iter().map(|t| (t.clone(), EdgeKind::Runtime)).collect();
            (from.clone(), to)
        })
        .collect();
    find_typed_cycles(&typed)
        .into_iter()
        .map(|c| c.files)
        .collect()
}

/// What it takes to close a cycle.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CycleKind {
    /// Runtime imports only: the files load each other when they run.
    Runtime,
    /// Needs an import that may load one of several files.
    Possible,
    /// Needs a type-only import, erased at runtime: harmless when the code
    /// runs, though it still ties the files together.
    TypeOnly,
}

impl CycleKind {
    pub fn label(self) -> &'static str {
        match self {
            CycleKind::Runtime => "runtime",
            CycleKind::Possible => "possible",
            CycleKind::TypeOnly => "type-only",
        }
    }
}

/// One group of files that import each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Cycle {
    /// The files in import order; the last imports the first.
    pub files: Vec<PathBuf>,
    pub kind: CycleKind,
}

/// [`find_cycles`] on typed edges, with the kind of each cycle.
///
/// Each group of files that reach each other through any edge is one
/// cycle. When runtime imports alone close a cycle inside the group, it is
/// a runtime cycle, shown as the shortest such cycle; otherwise, when
/// runtime and possible edges do, it is possible; otherwise it needs a
/// type-only import and is shown as the group's shortest cycle.
pub fn find_typed_cycles(graph: &TypedGraph) -> Vec<Cycle> {
    // Index nodes in sorted order so the result does not depend on hash order.
    let nodes: BTreeSet<&PathBuf> = graph
        .iter()
        .flat_map(|(from, to)| std::iter::once(from).chain(to.iter().map(|(t, _)| t)))
        .collect();
    let nodes: Vec<&PathBuf> = nodes.into_iter().collect();
    let index: HashMap<&PathBuf, usize> = nodes.iter().enumerate().map(|(i, p)| (*p, i)).collect();
    let edges_where = |keep: &dyn Fn(EdgeKind) -> bool| -> Vec<Vec<usize>> {
        nodes
            .iter()
            .map(|p| {
                let mut out: Vec<usize> = graph
                    .get(*p)
                    .map(|to| {
                        to.iter()
                            .filter(|(_, k)| keep(*k))
                            .map(|(t, _)| index[t])
                            .collect()
                    })
                    .unwrap_or_default();
                out.sort_unstable();
                out.dedup();
                out
            })
            .collect()
    };
    let all = edges_where(&|_| true);
    let runtime = edges_where(&|k| k == EdgeKind::Runtime);
    let not_type = edges_where(&|k| k != EdgeKind::Type);
    // The groups under the narrower edges that hold a cycle, and for each
    // node the index of its group.
    let cyclic_groups = |edges: &[Vec<usize>]| -> (Vec<Vec<usize>>, HashMap<usize, usize>) {
        let mut groups = Vec::new();
        let mut of = HashMap::new();
        for group in strongly_connected(edges) {
            let first = group[0];
            if group.len() == 1 && !edges[first].contains(&first) {
                continue;
            }
            for &member in &group {
                of.insert(member, groups.len());
            }
            groups.push(group);
        }
        (groups, of)
    };
    let runtime_groups = cyclic_groups(&runtime);
    let possible_groups = cyclic_groups(&not_type);

    let mut cycles: Vec<Cycle> = strongly_connected(&all)
        .into_iter()
        .filter_map(|group| {
            let start = *group.iter().min()?;
            if group.len() == 1 && !all[start].contains(&start) {
                return None;
            }
            let narrowest = [
                (CycleKind::Runtime, &runtime_groups, &runtime),
                (CycleKind::Possible, &possible_groups, &not_type),
            ]
            .into_iter()
            .find_map(|(kind, groups, edges)| {
                // The narrower group with the lowest-sorted file.
                let (groups, of) = groups;
                let sub = group
                    .iter()
                    .filter_map(|m| of.get(m))
                    .map(|&g| &groups[g])
                    .min_by_key(|g| g.iter().min())?;
                let start = *sub.iter().min()?;
                let members: HashSet<usize> = sub.iter().copied().collect();
                Some((kind, shortest_cycle(edges, &members, start)?))
            });
            let (kind, path) = match narrowest {
                Some(found) => found,
                None => {
                    let members: HashSet<usize> = group.into_iter().collect();
                    (CycleKind::TypeOnly, shortest_cycle(&all, &members, start)?)
                }
            };
            Some(Cycle {
                files: path.into_iter().map(|i| nodes[i].clone()).collect(),
                kind,
            })
        })
        .collect();

    cycles.sort_by(|a, b| {
        a.files
            .len()
            .cmp(&b.files.len())
            .then_with(|| a.files.cmp(&b.files))
    });
    cycles
}

/// The import edges of a cycle, including the one back to the start.
pub fn cycle_edges(cycle: &[PathBuf]) -> impl Iterator<Item = (&PathBuf, &PathBuf)> {
    cycle
        .iter()
        .zip(cycle.iter().cycle().skip(1))
        .take(cycle.len())
}

/// Tarjan's algorithm without recursion, so deep import chains cannot
/// overflow the stack.
fn strongly_connected(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let n = edges.len();
    let mut order = vec![UNSEEN; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut groups = Vec::new();
    let mut next = 0;

    for root in 0..n {
        if order[root] != UNSEEN {
            continue;
        }
        // (node, index of the next edge to follow)
        let mut work = vec![(root, 0)];
        order[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;

        while let Some(&mut (node, ref mut edge)) = work.last_mut() {
            if let Some(&to) = edges[node].get(*edge) {
                *edge += 1;
                if order[to] == UNSEEN {
                    order[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    work.push((to, 0));
                } else if on_stack[to] {
                    low[node] = low[node].min(order[to]);
                }
                continue;
            }

            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == order[node] {
                let mut group = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    group.push(member);
                    if member == node {
                        break;
                    }
                }
                groups.push(group);
            }
        }
    }
    groups
}

/// Shortest path from `start` back to itself inside `members` (breadth
/// first; ties go to the lowest-sorted file).
fn shortest_cycle(
    edges: &[Vec<usize>],
    members: &HashSet<usize>,
    start: usize,
) -> Option<Vec<usize>> {
    let mut previous: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(node) = queue.pop_front() {
        for &to in &edges[node] {
            if !members.contains(&to) {
                continue;
            }
            if to == start {
                let mut path = vec![node];
                let mut at = node;
                while at != start {
                    at = previous[&at];
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            if let std::collections::hash_map::Entry::Vacant(e) = previous.entry(to) {
                e.insert(node);
                queue.push_back(to);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[(&str, &[&str])]) -> HashMap<PathBuf, Vec<PathBuf>> {
        edges
            .iter()
            .map(|(from, to)| (PathBuf::from(from), to.iter().map(PathBuf::from).collect()))
            .collect()
    }

    fn names(cycles: &[Vec<PathBuf>]) -> Vec<Vec<String>> {
        cycles
            .iter()
            .map(|c| c.iter().map(|p| p.display().to_string()).collect())
            .collect()
    }

    #[test]
    fn finds_no_cycle_in_a_tree() {
        let g = graph(&[("a", &["b", "c"]), ("b", &["c"]), ("c", &[])]);
        assert!(find_cycles(&g).is_empty());
    }

    #[test]
    fn finds_two_and_three_file_cycles() {
        let g = graph(&[
            ("a", &["b"]),
            ("b", &["a"]),
            ("x", &["y"]),
            ("y", &["z"]),
            ("z", &["x"]),
            ("main", &["a", "x"]),
        ]);
        assert_eq!(
            names(&find_cycles(&g)),
            vec![vec!["a", "b"], vec!["x", "y", "z"]]
        );
    }

    #[test]
    fn reports_the_shortest_cycle_in_a_tangled_group() {
        // a → b → c → d → a, plus a shortcut c → a.
        let g = graph(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &["d", "a"]),
            ("d", &["a"]),
        ]);
        assert_eq!(names(&find_cycles(&g)), vec![vec!["a", "b", "c"]]);
    }

    #[test]
    fn finds_a_file_that_imports_itself() {
        let g = graph(&[("a", &["a"]), ("b", &["a"])]);
        assert_eq!(names(&find_cycles(&g)), vec![vec!["a"]]);
    }

    #[test]
    fn handles_long_chains_without_recursion() {
        let n = 50_000;
        let mut g: HashMap<PathBuf, Vec<PathBuf>> = (0..n)
            .map(|i| {
                (
                    PathBuf::from(format!("f{i:05}")),
                    vec![PathBuf::from(format!("f{:05}", i + 1))],
                )
            })
            .collect();
        g.insert(
            PathBuf::from(format!("f{n:05}")),
            vec![PathBuf::from("f00000")],
        );
        let cycles = find_cycles(&g);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), n + 1);
    }

    #[test]
    fn lists_cycle_edges_back_to_the_start() {
        let cycle = vec![PathBuf::from("a"), PathBuf::from("b")];
        let edges: Vec<_> = cycle_edges(&cycle).collect();
        assert_eq!(
            edges,
            vec![
                (&PathBuf::from("a"), &PathBuf::from("b")),
                (&PathBuf::from("b"), &PathBuf::from("a"))
            ]
        );
    }
}
