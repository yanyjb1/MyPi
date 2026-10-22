//! Short number forms. English groups by three; Chinese by four.

use crate::server::api::types::UsageLang;

pub fn compact(n: u64, lang: UsageLang) -> String {
    let units: &[(u64, &str)] = match lang {
        UsageLang::En => &[
            (1_000_000_000_000, "T"),
            (1_000_000_000, "B"),
            (1_000_000, "M"),
            (1_000, "K"),
        ],
        UsageLang::Zh => &[
            (10_000_000_000_000_000, "京"),
            (1_000_000_000_000, "兆"),
            (100_000_000, "亿"),
            (10_000, "万"),
        ],
    };
    for (scale, suffix) in units {
        if n >= *scale {
            return format!("{:.1}{suffix}", n as f64 / *scale as f64);
        }
    }
    n.to_string()
}
