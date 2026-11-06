//! Where the context stands.
//!
//! A request reports the prompt it carried, so a measured answer needs nothing but the last turn.
//! A compaction is the one gap: the next prompt is the summary plus the retained tail, and only
//! those two parts are known. The tail is estimated from the turns the truncation cut into — every
//! request reports how much its own blocks added, so a turn's share is its intervals' weight.

use std::collections::HashMap;

use serde_json::Value;

use crate::server::agent::r#loop::Partial;

/// The context in use, and whether that number is an estimate.
pub(crate) fn context(
    turns: &[Value],
    list: &[Value],
    running: Option<&Partial>,
    baseline: Option<u64>,
) -> (Option<u64>, bool) {
    // A lap just landed: its prompt is the context, to the token.
    if let Some(partial) = running {
        return (Some(partial.context), false);
    }
    let by_turn: HashMap<i64, &Value> = turns
        .iter()
        .filter_map(|row| row["turn_id"].as_i64().map(|id| (id, row)))
        .collect();

    // The newest turn that measured anything settles it: its prompt is the context.
    for row in list.iter().rev() {
        if row["is_compaction"].as_bool() == Some(true) {
            break;
        }
        let Some(turn) = by_turn.get(&row["turn_id"].as_i64().unwrap_or_default()) else {
            continue;
        };
        if let Some(prompt) = turn["prompt_tokens"].as_u64() {
            let completion = turn["completion_tokens"].as_u64().unwrap_or(0);
            return (Some(prompt + completion), false);
        }
    }

    // No measurement since the last compaction: estimate it, or fall back to the archive's floor.
    match estimate(list, &by_turn, baseline) {
        Some(tokens) => (Some(tokens), true),
        None => (baseline, baseline.is_some()),
    }
}

/// The summary plus the retained tail, plus the archive's fixed floor.
fn estimate(list: &[Value], by_turn: &HashMap<i64, &Value>, baseline: Option<u64>) -> Option<u64> {
    let at = list
        .iter()
        .rposition(|row| row["is_compaction"].as_bool() == Some(true))?;
    let turn_id = list[at]["turn_id"].as_i64()?;
    let summary = by_turn.get(&turn_id)?["completion_tokens"].as_u64()?;

    // The tail: every borrowed round after the compaction carries the intervals it kept.
    let mut kept: Vec<(i64, i64)> = Vec::new();
    for row in &list[at + 1..] {
        let Some(id) = row["turn_id"].as_i64() else {
            continue;
        };
        let Some(turn) = by_turn.get(&id) else {
            continue;
        };
        if turn["stop_reason"].as_str() != Some("borrowed") {
            continue;
        }
        kept.extend(intervals(&turn["ranges"]));
    }

    // What each pre-compaction turn added, weighted by how much of it the tail kept.
    let mut tail = 0.0;
    let mut previous: Option<(u64, u64)> = None;
    for row in &list[..at] {
        let Some(id) = row["turn_id"].as_i64() else {
            continue;
        };
        let Some(turn) = by_turn.get(&id) else {
            continue;
        };
        let (Some(prompt), Some(completion)) = (
            turn["prompt_tokens"].as_u64(),
            turn["completion_tokens"].as_u64(),
        ) else {
            continue;
        };
        let added = match previous {
            Some((before_prompt, before_completion)) => {
                prompt.saturating_sub(before_prompt + before_completion)
            }
            // Nothing to difference against: the first prompt is the floor plus its own blocks.
            None => prompt.saturating_sub(baseline.unwrap_or(0)),
        };
        if let Some(share) = weight(&intervals(&turn["ranges"]), &kept) {
            tail += added as f64 * share;
        }
        previous = Some((prompt, completion));
    }

    Some(baseline.unwrap_or(0) + summary + tail.round() as u64)
}

/// How much of `turn` a set of kept intervals covers, 0..=1 by block count.
fn weight(turn: &[(i64, i64)], kept: &[(i64, i64)]) -> Option<f64> {
    let size: i64 = turn.iter().map(|(lo, hi)| hi - lo + 1).sum();
    if size <= 0 {
        return None;
    }
    let held: i64 = turn
        .iter()
        .map(|(lo, hi)| {
            kept.iter()
                .map(|(klo, khi)| (hi.min(khi) - lo.max(klo) + 1).max(0))
                .sum::<i64>()
        })
        .sum();
    Some(held as f64 / size as f64)
}

/// `[[lo, hi], ...]` as pairs.
fn intervals(raw: &Value) -> Vec<(i64, i64)> {
    raw.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let pair = item.as_array()?;
                    Some((pair.first()?.as_i64()?, pair.get(1)?.as_i64()?))
                })
                .collect()
        })
        .unwrap_or_default()
}
