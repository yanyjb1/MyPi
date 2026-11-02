//! Money with a currency symbol; precision follows magnitude.

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
