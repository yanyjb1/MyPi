//! Relative durations for background jobs (`10min`, `2h`, `1d`, `1h30m`).

use anyhow::{Result, bail};

/// Parse one relative duration. No absolute timestamps, no signs.
pub(crate) fn parse(text: &str) -> Result<std::time::Duration> {
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "empty duration");
    let mut total = 0u64;
    let mut rest = text;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        anyhow::ensure!(digits > 0, "duration needs a number before `{rest}`");
        let number: u64 = rest[..digits].parse()?;
        rest = &rest[digits..];
        let unit_len = rest
            .find(|c: char| c.is_ascii_digit())
            .unwrap_or(rest.len());
        let unit = rest[..unit_len].trim();
        anyhow::ensure!(!unit.is_empty(), "duration needs a unit");
        let seconds = match unit {
            "s" | "sec" | "secs" => 1,
            "m" | "min" | "mins" => 60,
            "h" | "hr" | "hrs" => 3600,
            "d" | "day" | "days" => 86_400,
            other => bail!("unknown duration unit `{other}`"),
        };
        total = total.saturating_add(number.saturating_mul(seconds));
        rest = &rest[unit_len..];
    }
    Ok(std::time::Duration::from_secs(total))
}
