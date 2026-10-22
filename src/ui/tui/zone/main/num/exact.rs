//! Exact numbers, grouped by the same base as the short form.

use crate::server::api::types::UsageLang;

pub fn exact(n: u64, lang: UsageLang) -> String {
    let digits = n.to_string();
    let width = digits.len();
    let step = match lang {
        UsageLang::En => 3,
        UsageLang::Zh => 4,
    };
    let mut out = String::with_capacity(width + width / step);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (width - i) % step == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
