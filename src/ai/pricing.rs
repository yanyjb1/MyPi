//! Pricing — locally converts usage token counts into money in real
//! time.
//!
//! Formula (unit prices per 1M tokens, from each model's cost section in
//! models.yml):
//!
//! ```text
//! cost = ((prompt - cached) * input + cached * cacheRead + completion * output) / 1e6
//! ```
//!
//! About `cacheWrite`: **it is not part of the calculation**. Server
//! usage only reports `cached_tokens` (the amount **read** from cache);
//! no field represents "written to cache", so writes are priced as plain
//! input. The `cacheWrite` field in models.yml is currently reserved —
//! kept for the day a gateway actually reports it.
//!

use crate::ai::config::Cost;
use crate::ai::types::Usage;

// Divisor for per-1M-tokens pricing.
const PER_MILLION: f64 = 1_000_000.0;

// Cost of one request (numeric value in `Cost`'s currency; the formula itself is currency-agnostic).
//
// Cache writes (`c.cacheWrite`) are **not included**: usage has no
// "written to cache" counter (the OpenAI spec only provides
// `cached_tokens`, which means reads). An early revision multiplied it
// by 0.0 — a term that was always zero and merely looked like it
// handled cache writes. Rather than keep that decoration, the omission
// is documented here.
pub fn cost(usage: &Usage, c: &Cost) -> f64 {
    let prompt = usage.prompt_tokens as f64;
    // Cache reads cannot exceed total input. Compatibility layers
    // occasionally emit dirty data (`cached_tokens > prompt_tokens`,
    // observed with cumulative counters stuffed in), so **clamp both
    // sides**:
    //
    // - unclamped `cache_read`: dirty data bills the cache tier at an
    //   inflated token count -> overcharge;
    // - unclamped `plain_input`: `prompt - cached` goes negative ->
    //   drags the total down -> undercharge.
    //
    // Clamping only one side still produces a wrong answer, hence the
    // explicit `min`.
    let cache_read = (usage.cached_tokens.unwrap_or(0) as f64).min(prompt);
    let plain_input = prompt - cache_read;
    let output = usage.completion_tokens as f64;

    (plain_input * c.input + cache_read * c.cache_read + output * c.output) / PER_MILLION
}

// Session accumulator. The TUI calls record() once per TurnDone.
#[derive(Debug, Default, Clone)]
pub struct CostTracker {
    // Accumulated cost (numeric value in the current currency).
    pub total: f64,
    // Latest prompt_tokens — the current context size (numerator of the statusline ctx gauge).
    pub last_prompt_tokens: u64,
}

impl CostTracker {
    pub fn record(&mut self, usage: &Usage, c: &Cost) {
        self.total += cost(usage, c);
        self.last_prompt_tokens = usage.prompt_tokens;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Cost {
        Cost {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    #[test]
    fn plain_input_output() {
        // 1M in @1 + 1M out @4 = 5
        let u = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 2_000_000,
            cached_tokens: None,
            reasoning_tokens: None,
        };
        let v = cost(&u, &c(1.0, 4.0, 0.02, 0.0));
        assert!((v - 5.0).abs() < 1e-9, "got {v}");
    }

    #[test]
    fn cache_read_pays_less() {
        // 500K input with 400K cache hits: 0.1M*0.8 + 0.4M*0.1 = 0.08+0.04 = 0.12
        let u = Usage {
            prompt_tokens: 500_000,
            completion_tokens: 0,
            total_tokens: 500_000,
            cached_tokens: Some(400_000),
            reasoning_tokens: None,
        };
        let v = cost(&u, &c(0.8, 2.7, 0.1, 1.25));
        assert!((v - 0.12).abs() < 1e-9, "got {v}");
    }

    #[test]
    fn cached_exceeding_prompt_does_not_go_negative() {
        // Some compatibility layers report cached_tokens larger than
        // prompt_tokens. Unclamped, plain_input goes negative and drags
        // the total down — a free-money bug.
        let u = Usage {
            prompt_tokens: 100,
            completion_tokens: 10,
            total_tokens: 110,
            cached_tokens: Some(999),
            reasoning_tokens: None,
        };
        let v = cost(&u, &c(1.0, 1.0, 0.5, 0.0));
        assert!(v >= 0.0, "cost must never be negative, got {v}");
        // All billed as cache reads: (100*0.5 + 10*1.0)/1e6
        assert!((v - 60.0 / 1e6).abs() < 1e-12, "got {v}");
    }

    #[test]
    fn cache_write_price_is_not_charged() {
        // Whatever cacheWrite is set to must not affect the result —
        // pinning this semantic so nobody assumes it is live.
        let u = Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 0,
            total_tokens: 1_000_000,
            cached_tokens: Some(0),
            reasoning_tokens: None,
        };
        let with = cost(&u, &c(1.0, 0.0, 0.0, 999.0));
        let without = cost(&u, &c(1.0, 0.0, 0.0, 0.0));
        assert_eq!(with, without);
        assert!((with - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tracker_accumulates() {
        let mut t = CostTracker::default();
        let u1 = Usage {
            prompt_tokens: 590,
            completion_tokens: 5,
            total_tokens: 595,
            cached_tokens: Some(0),
            reasoning_tokens: None,
        };
        t.record(&u1, &c(0.8, 2.7, 0.1, 1.25));
        assert_eq!(t.last_prompt_tokens, 590);
        assert!(t.total > 0.0);
    }
}
