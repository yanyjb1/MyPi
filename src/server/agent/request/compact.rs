//! Compaction: where to cut, what the summary request carries, and what may be kept.

use anyhow::{Result, bail};

use super::contract::{Borrow, Retention, TurnView};
use super::message::Message;

/// The cut: the region a summary replaces, and the intervals that survive verbatim.
#[derive(Debug, Default)]
pub struct Plan {
    pub region: Vec<Message>,
    pub borrows: Vec<Borrow>,
}

/// Cut from the newest block backwards, whole blocks at a time. The newest block always stays.
pub fn plan(turns: &[TurnView], retention: Retention) -> Plan {
    let flat: Vec<(i64, Message)> = turns
        .iter()
        .flat_map(|turn| {
            turn.pieces
                .iter()
                .map(|piece| (piece.block, piece.message.clone()))
        })
        .collect();

    let mut kept_chars = 0usize;
    let mut kept_blocks = 0usize;
    let mut cut = None;
    for (block, message) in flat.iter().rev() {
        let fits = match retention {
            Retention::Chars(budget) => kept_blocks == 0 || kept_chars + chars(message) <= budget,
            Retention::Blocks(want) => kept_blocks == 0 || kept_blocks < want.max(1),
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
    for (block, message) in &flat {
        if *block < cut {
            region.push(message.clone());
        }
    }

    let borrows = turns
        .iter()
        .filter_map(|turn| {
            let ranges = clip(&turn.ranges, cut);
            (!ranges.is_empty()).then_some(Borrow { ranges })
        })
        .collect();

    Plan { region, borrows }
}

/// Keep what lies at or after the cut. A range straddling the cut starts at it.
fn clip(ranges: &[[i64; 2]], cut: i64) -> Vec<[i64; 2]> {
    ranges
        .iter()
        .filter_map(|[start, end]| {
            if *end < cut {
                None
            } else if *start < cut {
                Some([cut, *end])
            } else {
                Some([*start, *end])
            }
        })
        .collect()
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
