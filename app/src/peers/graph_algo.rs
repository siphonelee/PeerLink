use core::cmp::Ordering;
use hashbrown::hash_map::{
    Entry::{Occupied, Vacant},
    HashMap,
};
use petgraph::{
    algo::Measure,
    visit::{EdgeRef as _, IntoEdges, VisitMap as _, Visitable},
};
use std::{collections::BinaryHeap, hash::Hash};

/// `MinScored<K, T>` holds a score `K` and a scored object `T` in
/// a pair for use with a `BinaryHeap`.
///
/// `MinScored` compares in reverse order by the score, so that we can
/// use `BinaryHeap` as a min-heap to extract the score-value pair with the
/// least score.
///
/// **Note:** `MinScored` implements a total order (`Ord`), so that it is
/// possible to use float types as scores.
#[derive(Copy, Clone, Debug)]
pub struct MinScored<K, T>(pub K, pub T);

impl<K: PartialOrd, T> PartialEq for MinScored<K, T> {
    #[inline]
    fn eq(&self, other: &MinScored<K, T>) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<K: PartialOrd, T> Eq for MinScored<K, T> {}

impl<K: PartialOrd, T> PartialOrd for MinScored<K, T> {
    #[inline]
    fn partial_cmp(&self, other: &MinScored<K, T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: PartialOrd, T> Ord for MinScored<K, T> {
    #[inline]
    fn cmp(&self, other: &MinScored<K, T>) -> Ordering {
        let a = &self.0;
        let b = &other.0;
        if a == b {
            Ordering::Equal
        } else if a < b {
            Ordering::Greater
        } else if a > b {
            Ordering::Less
        } else if a.ne(a) && b.ne(b) {
            // these are the NaN cases
            Ordering::Equal
        } else if a.ne(a) {
            // Order NaN less, so that it is last in the MinScore order
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

pub type DijkstraResult<K, NodeId> = (HashMap<NodeId, K>, HashMap<NodeId, (NodeId, usize)>);

pub fn dijkstra_with_first_hop<G, F, K>(
    graph: G,
    start: G::NodeId,
    mut edge_cost: F,
) -> DijkstraResult<K, G::NodeId>
where
    G: IntoEdges + Visitable,
    G::NodeId: Eq + Hash + Clone,
    F: FnMut(G::EdgeRef) -> K,
    K: Measure + Copy,
{
    let mut visited = graph.visit_map();
    let mut scores = HashMap::new();
    let mut first_hop = HashMap::new();
    let mut visit_next = BinaryHeap::new();
    let zero_score = K::default();
    scores.insert(start, zero_score);
    visit_next.push(MinScored(zero_score, start));
    first_hop.insert(start, (start, 0));

    while let Some(MinScored(node_score, node)) = visit_next.pop() {
        if visited.is_visited(&node) {
            continue;
        }
        for edge in graph.edges(node) {
            let next = edge.target();
            if visited.is_visited(&next) {
                continue;
            }
            let next_score = node_score + edge_cost(edge);
            match scores.entry(next) {
                Occupied(mut ent) => {
                    if next_score < *ent.get() {
                        *ent.get_mut() = next_score;
                        visit_next.push(MinScored(next_score, next));
                        // 继承前驱的 first_hop，或自己就是第一跳
                        let hop = if node == start {
                            (next, 0)
                        } else {
                            first_hop[&node]
                        };
                        first_hop.insert(next, (hop.0, hop.1 + 1));
                    }
                }
                Vacant(ent) => {
                    ent.insert(next_score);
                    visit_next.push(MinScored(next_score, next));
                    let hop = if node == start {
                        (next, 0)
                    } else {
                        first_hop[&node]
                    };
                    first_hop.insert(next, (hop.0, hop.1 + 1));
                }
            }
        }
        visited.visit(node);
    }

    (scores, first_hop)
}
