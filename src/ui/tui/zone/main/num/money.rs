//! Money with a currency symbol; precision follows magnitude, then column budget.

pub fn money(v: f64, symbol: &str) -> String {
    if !v.is_finite() || v <= 0.0 {
        return format!("{symbol}0.00");
    }
    if v >= 0.01 {
        return format!("{symbol}{v:.2}");
    }
    let digits = (-(v.log10().floor()) as i32 + 1).clamp(2, 8) as usize;
    format!("{symbol}{v:.digits$}")
}

pub fn money_within(v: f64, symbol: &str, max_w: usize) -> String {
    let full = money(v, symbol);
    if full.chars().count() <= max_w {
        return full;
    }
    let two = format!("{symbol}{v:.2}");
    if two.chars().count() <= max_w {
        return two;
    }
    format!("{symbol}{v:.0}")
}
