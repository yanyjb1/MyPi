//! Project the session structure into tree rows.

use std::collections::{BTreeMap, HashMap};

use super::layout::{self, ForkRef, NodeRef};
use super::rows::Row;
use crate::server::storage::BlockKind;
use crate::ui::session::{BlockRef, View};

struct Node {
    turn: i64,
    kind: String,
    text: String,
    body: Option<String>,
}

/// Rows in tree order. `only_user` keeps user messages.
pub fn rows(view: &View<'_>, only_user: bool) -> Vec<Row> {
    let mut held: HashMap<i64, BlockRef<'_>> = HashMap::new();
    for block in view.chain() {
        if let Some(id) = block.id() {
            held.insert(id, block);
        }
    }

    let mut nodes: BTreeMap<i64, (i64, Option<BlockRef<'_>>)> = BTreeMap::new();
    for shape in view.shapes.iter() {
        for block in &shape.blocks {
            nodes
                .entry(block.id)
                .or_insert((shape.turn, held.get(&block.id).copied()));
        }
    }
    for block in view.blocks.iter() {
        nodes.entry(block.block.id).or_insert((
            block.turn.unwrap_or(0),
            Some(BlockRef::Landed(&block.block)),
        ));
    }

    let session = view.attached.unwrap_or(0);
    let mut selected: Vec<(i64, Node)> = Vec::new();
    for (id, (turn, block)) in nodes {
        let kind = block.map(|b| role(b.kind())).unwrap_or("block");
        if only_user && kind != "user" {
            continue;
        }
        let text = block
            .map(summary)
            .unwrap_or_else(|| format!("{kind}: #{id}"));
        let body = block.and_then(|b| match b.kind() {
            BlockKind::User => b.content().cloned().or_else(|| b.text().cloned()),
            _ => None,
        });
        selected.push((
            id,
            Node {
                turn,
                kind: kind.to_string(),
                text,
                body,
            },
        ));
    }

    let refs: Vec<NodeRef> = selected
        .iter()
        .map(|(id, _)| NodeRef {
            block: *id,
            session,
        })
        .collect();
    let forks = [ForkRef {
        id: session,
        parent: None,
        fork_block: None,
    }];
    let placed = layout::rows(&forks, &refs);
    placed
        .into_iter()
        .filter_map(|p| {
            let (id, node) = selected.get(p.src)?;
            Some(Row {
                geom: p.geom,
                block_id: *id,
                turn: node.turn,
                text: node.text,
                body: node.body,
                fork: 0,
                kind: node.kind.clone(),
            })
        })
        .collect()
}

/// The range the tree still needs: every shaped block, full span.
pub fn range(view: &View<'_>) -> Option<(i64, i64)> {
    let mut start = i64::MAX;
    let mut end = i64::MIN;
    for shape in view.shapes.iter() {
        for block in &shape.blocks {
            start = start.min(block.id);
            end = end.max(block.id);
        }
    }
    (start <= end).then_some((start, end.saturating_add(1)))
}

/// The row the history's newest landed block sits on.
pub fn current(view: &View<'_>, rows: &[Row]) -> usize {
    let Some(last) = view.blocks.back().map(|b| b.block.id) else {
        return 0;
    };
    rows.iter()
        .position(|row| row.block_id == last)
        .unwrap_or(0)
}

fn role(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::User => "user",
        BlockKind::Assistant => "assistant",
        BlockKind::Reasoning => "reasoning",
        BlockKind::ToolRequest => "tool_call",
        BlockKind::ToolResult => "tool_result",
    }
}

fn summary(block: BlockRef<'_>) -> String {
    let kind = role(block.kind());
    let body = block
        .content()
        .or_else(|| block.text())
        .or_else(|| block.result())
        .or_else(|| block.args())
        .map(String::as_str)
        .unwrap_or("");
    let first = body.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        format!("{kind}: #{}", block.id().unwrap_or_default())
    } else {
        format!("{kind}: {first}")
    }
}
