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
//! Only the imports the dependency graph tracks are seen, and type-only
//! imports count too, so a finding is something to check, not proof of a
//! runtime problem.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// Find circular imports in `graph`.
///
/// Returns one cycle per group of files that import each other, as the
/// files in import order starting from the group's first path; the last
/// file imports the first. A file that imports itself is a cycle of one.
/// Cycles are sorted shortest first, then by path, so output is stable.
pub fn find_cycles(graph: &HashMap<PathBuf, Vec<PathBuf>>) -> Vec<Vec<PathBuf>> {
    // Index nodes in sorted order so the result does not depend on hash order.
    let nodes: BTreeSet<&PathBuf> = graph
        .iter()
        .flat_map(|(from, to)| std::iter::once(from).chain(to.iter()))
        .collect();
    let nodes: Vec<&PathBuf> = nodes.into_iter().collect();
    let index: HashMap<&PathBuf, usize> = nodes.iter().enumerate().map(|(i, p)| (*p, i)).collect();
    let edges: Vec<Vec<usize>> = nodes
        .iter()
        .map(|p| {
            let mut out: Vec<usize> = graph
                .get(*p)
                .map(|to| to.iter().map(|t| index[t]).collect())
                .unwrap_or_default();
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();

    let mut cycles: Vec<Vec<PathBuf>> = strongly_connected(&edges)
        .into_iter()
        .filter_map(|group| {
            let start = *group.iter().min()?;
            if group.len() == 1 && !edges[start].contains(&start) {
                return None;
            }
            let members: HashSet<usize> = group.into_iter().collect();
            let path = shortest_cycle(&edges, &members, start)?;
            Some(path.into_iter().map(|i| nodes[i].clone()).collect())
        })
        .collect();

    cycles.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
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
