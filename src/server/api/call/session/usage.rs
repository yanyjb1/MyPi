//! The usage bill: tokens per model, priced here from the stored tokens and the config's rates.

use anyhow::Result;
use rusqlite::Connection;

use crate::server::api::call::ctx::Ctx;
use crate::server::config::Config;
use crate::server::storage::RoundUsage;

pub use crate::ipc::contract::{ModelUsage, UsageCost, UsageLang, UsageReply};

/// What one model charges, per million tokens.
#[derive(Debug, Clone, PartialEq, Default)]
struct PriceSheet {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
    symbol: String,
}

/// One model's tokens for a session.
#[derive(Debug, Clone, PartialEq)]
struct Rated {
    provider: Option<String>,
    model: String,
    rounds: u64,
    usage: RoundUsage,
    sheet: Option<PriceSheet>,
}

pub(crate) fn answer(ctx: &Ctx, session: i64, lang: UsageLang) -> Result<UsageReply> {
    Ok(UsageReply {
        lang,
        groups: bill_for(ctx.cfg, ctx.reader, session)?,
    })
}

fn tokens_by_model(conn: &Connection, session: i64) -> Result<Vec<Rated>> {
    let mut stmt = conn.prepare(
        "SELECT r.provider, r.model, COUNT(*),
                COALESCE(SUM(t.prompt_tokens), 0),
                COALESCE(SUM(t.completion_tokens), 0),
                COALESCE(SUM(t.cached_tokens), 0),
                COALESCE(SUM(t.reasoning_tokens), 0),
                COALESCE(SUM(t.cache_write_tokens), 0)
           FROM turns t JOIN session_turns s ON s.turn_id = t.turn_id
           JOIN request_meta r ON r.request_id = t.request_id
          WHERE s.session_id = ?1
          GROUP BY r.provider, r.model
          ORDER BY MIN(s.idx)",
    )?;
    let found = stmt.query_map([session], |r| {
        Ok(Rated {
            provider: r.get(0)?,
            model: r.get(1)?,
            rounds: r.get::<_, i64>(2)?.max(0) as u64,
            usage: RoundUsage {
                prompt_tokens: r.get::<_, i64>(3)?.max(0) as u64,
                completion_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                cached_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                reasoning_tokens: r.get::<_, i64>(6)?.max(0) as u64,
                cache_write_tokens: r.get::<_, i64>(7)?.max(0) as u64,
            },
            sheet: None,
        })
    })?;
    found
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn bill_for(cfg: &Config, conn: &Connection, session: i64) -> Result<Vec<ModelUsage>> {
    let mut rated = tokens_by_model(conn, session)?;
    for r in &mut rated {
        r.sheet = sheet_of(cfg, r);
    }
    Ok(bill(&rated))
}

fn sheet_of(cfg: &Config, r: &Rated) -> Option<PriceSheet> {
    let provider = r.provider.as_deref()?;
    let rm = cfg.model_by_id(&format!("{provider}:{}", r.model)).ok()?;
    let cost = rm.entry.cost;
    if cost.input == 0.0 && cost.output == 0.0 && cost.cache_read == 0.0 && cost.cache_write == 0.0
    {
        return None;
    }
    let fx = &cfg.app.tui.fx;
    let (input, symbol) = fx.to_display(cost.input, rm.entry.currency);
    let (output, _) = fx.to_display(cost.output, rm.entry.currency);
    let (cache_read, _) = fx.to_display(cost.cache_read, rm.entry.currency);
    let (cache_write, _) = fx.to_display(cost.cache_write, rm.entry.currency);
    Some(PriceSheet {
        input,
        output,
        cache_read,
        cache_write,
        symbol: symbol.to_string(),
    })
}

fn bill(rated: &[Rated]) -> Vec<ModelUsage> {
    rated
        .iter()
        .map(|r| ModelUsage {
            id: r.model.clone(),
            provider: r.provider.clone(),
            rounds: r.rounds,
            prompt_tokens: r.usage.prompt_tokens,
            completion_tokens: r.usage.completion_tokens,
            cached_tokens: r.usage.cached_tokens,
            reasoning_tokens: r.usage.reasoning_tokens,
            cache_write_tokens: r.usage.cache_write_tokens,
            cost: r.sheet.as_ref().map(|s| cost_of(s, r.usage)),
            currency: r.sheet.as_ref().map(|s| s.symbol.clone()),
        })
        .collect()
}

fn cost_of(sheet: &PriceSheet, u: RoundUsage) -> UsageCost {
    const PER_MILLION: f64 = 1_000_000.0;
    let cached = u.cached_tokens.min(u.prompt_tokens);
    let written = u
        .cache_write_tokens
        .min(u.prompt_tokens.saturating_sub(cached));
    let fresh = u
        .prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(written);
    UsageCost {
        input: fresh as f64 * sheet.input / PER_MILLION,
        output: u.completion_tokens as f64 * sheet.output / PER_MILLION,
        cache_read: cached as f64 * sheet.cache_read / PER_MILLION,
        cache_write: written as f64 * sheet.cache_write / PER_MILLION,
    }
}
