//! Short number forms. `en` groups by thousands, `zh` by myriads.

use super::Lang;

pub fn compact(n: u64, lang: Lang) -> String {
    match lang {
        Lang::En => en(n),
        Lang::Zh => zh(n),
    }
}

fn en(n: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "T"),
        (1_000_000_000, "B"),
        (1_000_000, "M"),
        (1_000, "K"),
    ];
    for (scale, suffix) in UNITS {
        if n >= scale {
            return format!("{:.1}{suffix}", n as f64 / scale as f64);
        }
    }
    n.to_string()
}

/// Below one myriad stays bare; 万, 亿 and 万亿 are the only named scales.
fn zh(n: u64) -> String {
    const WAN: u64 = 10_000;
    const YI: u64 = 100_000_000;
    const WANYI: u64 = 1_000_000_000_000;
    if n >= WANYI {
        return format!("{:.1}万亿", n as f64 / WANYI as f64);
    }
    if n >= YI {
        return format!("{:.1}亿", n as f64 / YI as f64);
    }
    if n >= WAN {
        return format!("{:.1}万", n as f64 / WAN as f64);
    }
    n.to_string()
}
