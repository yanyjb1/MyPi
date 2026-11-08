//! `usage` — tokens per model, the cache-hit share, what it cost, and where the context stands.
//!
//! The session's bill is the union of stamped blocks; the running turn's laps arrive from memory.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{json, Value};

use crate::server::api::{Api, Refuse};
use crate::server::config::ResolvedModel;
use crate::server::storage::Reader;

use super::estimate;

#[derive(Default)]
struct Tally {
    rounds: i64,
    prompt: i64,
    completion: i64,
    cached: i64,
    written: i64,
}

/// One group's bill, in the model's own currency and in both display currencies.
struct Quote {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
    total: f64,
    cny: f64,
    usd: f64,
    currency: &'static str,
}

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let turns = reader.turns(session)?;
    let list = reader.list(session)?;
    let baseline = baseline_prompt(&reader, session);

    let mut groups: BTreeMap<String, Tally> = BTreeMap::new();
    for turn in &turns {
        let Some((provider, model)) = pair_of(turn) else {
            continue;
        };
        groups
            .entry(format!("{provider}:{model}"))
            .or_default()
            .rounds += 1;
    }

    // One stamped block bills one lap, priced at the turn found by interval coverage.
    let mut stamps: Vec<estimate::Stamp> = Vec::new();
    for (turn, span) in spans(&turns) {
        for block in reader.blocks(&[span])? {
            let Some(prompt) = block["usage_prompt"].as_i64() else {
                continue;
            };
            let prompt = prompt.max(0) as u64;
            let completion = block["usage_completion"].as_i64().unwrap_or(0).max(0) as u64;
            stamps.push(estimate::Stamp {
                at: block["id"].as_i64().unwrap_or(0),
                prompt,
                completion,
            });
            let address = pair_of(turn)
                .map(|(provider, model)| format!("{provider}:{model}"))
                .unwrap_or_default();
            let tally = groups.entry(address).or_default();
            tally.prompt += prompt as i64;
            tally.completion += completion as i64;
            tally.cached += block["usage_cached"].as_i64().unwrap_or(0);
            tally.written += block["usage_cache_write"].as_i64().unwrap_or(0);
        }
    }
    // The running turn is not stored yet: its laps arrive here, priced at the model in use.
    let running = api.ledger().running(session);
    if let Some((partial, (provider, model))) = &running {
        let tally = groups.entry(format!("{provider}:{model}")).or_default();
        tally.prompt += partial.usage.prompt as i64;
        tally.completion += partial.usage.completion as i64;
        tally.cached += partial.usage.cached as i64;
        tally.written += partial.usage.cache_write as i64;
    }

    let cfg = api.cfg();
    let fx = cfg
        .get_json("config.ui.tui.fx.usdCny", None)
        .ok()
        .and_then(|value| value.as_f64())
        .filter(|rate| *rate > 0.0)
        .unwrap_or(6.0);

    let mut prompt = 0i64;
    let mut completion = 0i64;
    let mut cached = 0i64;
    let mut written = 0i64;
    let mut cny = 0.0;
    let mut usd = 0.0;
    for tally in groups.values() {
        prompt += tally.prompt;
        completion += tally.completion;
        cached += tally.cached;
        written += tally.written;
    }
    let total = prompt + completion;
    let mut rows: Vec<Value> = Vec::with_capacity(groups.len());
    for (address, tally) in &groups {
        let (provider, model) = address.split_once(':').unwrap_or(("", address));
        let quote = cfg
            .model_by_id(address)
            .map(|resolved| quote(&resolved, tally, fx));
        if let Some(quote) = &quote {
            cny += quote.cny;
            usd += quote.usd;
        }
        rows.push(json!({
            "address": address,
            "provider": provider,
            "model": model,
            "rounds": tally.rounds,
            "prompt_tokens": tally.prompt,
            "completion_tokens": tally.completion,
            "cached_tokens": tally.cached,
            "cache_write_tokens": tally.written,
            "share": share(tally.prompt + tally.completion, total),
            "price": quote.map(|quote| json!({
                "currency": quote.currency,
                "input": quote.input,
                "output": quote.output,
                "cache_read": quote.cache_read,
                "cache_write": quote.cache_write,
                "total": quote.total,
                "cny": quote.cny,
                "usd": quote.usd,
            })),
        }));
    }
    rows.sort_by(|left, right| {
        let share = |row: &Value| row["share"].as_f64().unwrap_or(0.0);
        share(right)
            .partial_cmp(&share(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let (context, estimated) = estimate::context(
        &list,
        running.as_ref().map(|(partial, _)| partial),
        &stamps,
        baseline,
    );

    Ok(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached,
        "cache_write_tokens": written,
        "total_tokens": total,
        "cache_hit_rate": share(cached, total),
        "cny": cny,
        "usd": usd,
        "context": context,
        "estimated": estimated,
        "baseline_prompt": baseline,
        "groups": rows,
    }))
}

/// `(provider, model)` of a round row, when it asked at all.
fn pair_of(turn: &Value) -> Option<(&str, &str)> {
    Some((turn["provider"].as_str()?, turn["model"].as_str()?))
}

/// Every round's block intervals, paired with its row, in turn order.
fn spans(turns: &[Value]) -> Vec<(&Value, [i64; 2])> {
    let mut out = Vec::new();
    for turn in turns {
        let Some(items) = turn["ranges"].as_array() else {
            continue;
        };
        for item in items {
            let Some(pair) = item.as_array() else {
                continue;
            };
            let (Some(lo), Some(hi)) = (
                pair.first().and_then(Value::as_i64),
                pair.get(1).and_then(Value::as_i64),
            ) else {
                continue;
            };
            if hi >= lo {
                out.push((turn, [lo, hi]));
            }
        }
    }
    out
}

/// The smallest context this session's request archive ever carried; `None` until one is known.
pub(crate) fn baseline_prompt(reader: &Reader, session: i64) -> Option<u64> {
    let meta = reader.session(session).ok()??["meta"].as_i64()?;
    reader.request(meta).ok()??["baseline_prompt"].as_u64()
}

/// What one group cost, in the model's own currency and in both display currencies.
fn quote(model: &ResolvedModel, tally: &Tally, fx: f64) -> Quote {
    let per_million = |tokens: i64, price: f64| tokens as f64 / 1_000_000.0 * price;
    let cost = model.entry.cost;
    let uncached = tally.prompt.saturating_sub(tally.cached);
    let input = per_million(uncached, cost.input);
    let output = per_million(tally.completion, cost.output);
    let read = per_million(tally.cached, cost.cache_read);
    let write = per_million(tally.written, cost.cache_write);
    let total = input + output + read + write;
    let (cny, usd) = match model.entry.currency {
        crate::server::config::Currency::CNY => (total, total / fx),
        crate::server::config::Currency::USD => (total * fx, total),
    };
    Quote {
        input,
        output,
        cache_read: read,
        cache_write: write,
        total,
        cny,
        usd,
        currency: model.entry.currency.as_str(),
    }
}

fn share(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        return 0.0;
    }
    part as f64 / whole as f64
}
