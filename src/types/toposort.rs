use crate::{error::Error, Result};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

pub fn kahn_deterministic(
    comp_edges: &[HashSet<usize>],
    mut indeg: Vec<usize>,
) -> Result<Vec<usize>> {
    let comp_n = comp_edges.len();

    // Deterministic tie-breaking: use a min-heap (Reverse) so selection doesn't depend on hash order.
    let mut heap: BinaryHeap<Reverse<usize>> = indeg
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| if d == 0 { Some(Reverse(i)) } else { None })
        .collect();

    let mut order = Vec::with_capacity(comp_n);
    while let Some(Reverse(c)) = heap.pop() {
        order.push(c);

        // comp_edges[c] is a HashSet; sort to keep deterministic traversal.
        let mut tos: Vec<usize> = comp_edges[c].iter().copied().collect();
        tos.sort_unstable();
        for to in tos {
            indeg[to] -= 1;
            if indeg[to] == 0 {
                heap.push(Reverse(to));
            }
        }
    }

    if order.len() != comp_n {
        return Err(Error::msg("internal error: cyclic component graph"));
    }

    Ok(order)
}
