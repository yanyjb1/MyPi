//! Exact numbers, grouped by three.

pub fn exact(n: u64) -> String {
    let digits = n.to_string();
    let width = digits.len();
    let mut out = String::with_capacity(width + width / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (width - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
