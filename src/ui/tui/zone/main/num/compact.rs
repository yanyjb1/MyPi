//! Short number forms, grouped by three.

pub fn compact(n: u64) -> String {
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
