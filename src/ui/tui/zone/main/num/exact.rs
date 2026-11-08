//! Exact numbers. `en` groups by three digits, `zh` by four.

use super::Lang;

pub fn exact(n: u64, lang: Lang) -> String {
    let digits = n.to_string();
    let width = digits.len();
    let group = match lang {
        Lang::En => 3,
        Lang::Zh => 4,
    };
    let mut out = String::with_capacity(width + width / group);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (width - i).is_multiple_of(group) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
