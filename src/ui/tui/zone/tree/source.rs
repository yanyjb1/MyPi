//! Project the structure and the blocks into tree rows.

use std::collections::BTreeMap;

use super::layout::{self, ForkRef, NodeRef};
use super::rows::Row;
use super::zone::Node;
use crate::ui::session::Structure;

/// Rows in tree order. `only_user` keeps user messages.
pub fn rows(structure: &Structure, blocks: &BTreeMap<i64, Node>, only_user: bool) -> Vec<Row> {
    // Every shaped block id, with the round that owns it; a block the window did not carry
    // still gets a row, only without a summary.
    let mut held: BTreeMap<i64, (i64, Option<&Node>)> = BTreeMap::new();
    for turn in structure.turns() {
        for [lo, hi] in &turn.ranges {
            for id in *lo..=*hi {
                held.entry(id).or_insert((turn.id, None));
            }
        }
    }
    for (id, node) in blocks {
        let turn = held.get(id).map(|(turn, _)| *turn).unwrap_or(0);
        held.insert(*id, (turn, Some(node)));
    }

    let selected: Vec<(i64, Row)> = held
        .into_iter()
        .filter(|(_, (_, node))| !only_user || node.is_some_and(|node| node.kind == "user"))
        .map(|(id, (turn, node))| {
            let row = match node {
                Some(node) => Row {
                    geom: Default::default(),
                    block_id: id,
                    turn,
                    text: node.text.clone(),
                    body: node.body.clone(),
                    fork: 0,
                    kind: node.kind.clone(),
                },
                None => Row {
                    geom: Default::default(),
                    block_id: id,
                    turn,
                    text: format!("block: #{id}"),
                    body: None,
                    fork: 0,
                    kind: "block".to_string(),
                },
            };
            (id, row)
        })
        .collect();

    let refs: Vec<NodeRef> = selected
        .iter()
        .map(|(id, _)| NodeRef {
            block: *id,
            session: 0,
        })
        .collect();
    let forks = [ForkRef {
        id: 0,
        parent: None,
        fork_block: None,
    }];
    let placed = layout::rows(&forks, &refs);
    placed
        .into_iter()
        .filter_map(|p| {
            let mut row = selected.get(p.src)?.1.clone();
            row.geom = p.geom;
            Some(row)
        })
        .collect()
}
