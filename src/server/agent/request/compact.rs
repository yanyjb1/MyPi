//! Compaction: where to cut, what the summary request carries, and what may be kept.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use super::contract::{Borrow, Retention, TurnView};
use super::message::Message;

/// The cut: the region a summary replaces, and the intervals that survive verbatim.
#[derive(Debug, Default)]
pub struct Plan {
    pub region: Vec<Message>,
    pub borrows: Vec<Borrow>,
}

/// Keep nothing: the whole history goes into the summary.
fn keeps_nothing(retention: Retention) -> bool {
    match retention {
        Retention::Chars(budget) => budget == 0,
        Retention::Blocks(want) => want == 0,
    }
}

/// Cut from the newest block backwards, whole blocks at a time. The newest block always stays,
/// unless the budget is zero: then nothing does. The round writing the compaction is not history
/// yet, so its own blocks are never counted or kept.
pub fn plan(turns: &[TurnView], retention: Retention, own: i64) -> Plan {
    let turns: Vec<&TurnView> = turns.iter().filter(|turn| turn.turn != own).collect();
    let flat: Vec<(i64, i64, Message)> = turns
        .iter()
        .flat_map(|turn| {
            turn.sent()
                .into_iter()
                .map(|piece| (turn.turn, piece.block, piece.message))
        })
        .collect();

    if keeps_nothing(retention) {
        return Plan {
            region: flat.into_iter().map(|(_, _, message)| message).collect(),
            borrows: Vec::new(),
        };
    }

    let mut kept_chars = 0usize;
    let mut kept_blocks = 0usize;
    let mut cut = None;
    for (_, block, message) in flat.iter().rev() {
        let fits = match retention {
            Retention::Chars(budget) => kept_blocks == 0 || kept_chars + chars(message) <= budget,
            Retention::Blocks(want) => kept_blocks == 0 || kept_blocks < want,
        };
        if !fits {
            break;
        }
        kept_chars += chars(message);
        kept_blocks += 1;

        cut = Some(*block);
    }

    let Some(cut) = cut else {
        return Plan::default();
    };

    let mut region = Vec::new();
    let mut kept: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for (turn, block, message) in &flat {
        if *block < cut {
            region.push(message.clone());
        } else {
            kept.entry(*turn).or_default().push(*block);
        }
    }

    // The intervals a turn keeps are the blocks it actually sends: a compaction round hands out
    // only its summary, so its instruction is never borrowed back.
    let borrows = kept
        .into_values()
        .map(|blocks| Borrow {
            ranges: runs(&blocks),
        })
        .collect();

    Plan { region, borrows }
}

/// Ascending block ids as closed intervals: adjacent ids share one.
fn runs(blocks: &[i64]) -> Vec<[i64; 2]> {
    let mut out: Vec<[i64; 2]> = Vec::new();
    for block in blocks {
        match out.last_mut() {
            Some(last) if last[1] + 1 == *block => last[1] = *block,
            _ => out.push([*block, *block]),
        }
    }
    out
}

/// How many characters a string holds; the only unit this domain measures in.
fn width(text: &str) -> usize {
    text.chars().count()
}

/// What one message weighs, in characters. The estimate is the only size this domain has.
pub fn chars(message: &Message) -> usize {
    match message {
        Message::User {
            prefix,
            content,
            suffix,
        } => {
            prefix.as_deref().map_or(0, width) + width(content) + suffix.as_deref().map_or(0, width)
        }
        Message::Assistant { content } => width(content),
        Message::Reasoning { content } => width(content),
        Message::Call { name, args, .. } => width(name) + width(args) + 32,
        Message::Result { result, .. } => width(result),
    }
}

pub fn region_chars(region: &[Message]) -> usize {
    region.iter().map(chars).sum()
}

/// A summary that is not smaller than the region it replaces is a net loss.
pub fn check_shrink(summary: &str, region: &[Message]) -> Result<()> {
    let before = region_chars(region);
    let after = width(summary);
    if after >= before {
        bail!(
            "the summary is not smaller than the region it replaces ({after}/{before} characters); the context is untouched"
        );
    }
    Ok(())
}

/// Fold the focus into the instruction; a missing focus drops the marker's whole line.
pub fn render_instruction(template: &str, focus: &str) -> String {
    if focus.trim().is_empty() {
        return template
            .lines()
            .filter(|line| !line.contains("{{focus}}"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    template.replace("{{focus}}", focus.trim())
}
