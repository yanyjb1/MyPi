//! 大数的写法 —— `/usage` 面板与状态栏量表共用这一套。
//!
//! 两套口径，切换的**不只是单位名，还有进位基数**：
//!
//! - **英文**：三位一进，缩写 `K` / `M` / `B` / `T`（1K = 1 000）。
//! - **中文**：四位一进，缩写 `万` / `亿` / `兆` / `京`（1万 = 1 0000），
//!   确切值也按四位分组（`244,0000`）。
//!
//! 这是刻意的：中文读数的断点就是万、亿、兆、京，用三位分组去数中文数字，读的
//! 人还得自己换算。缩写与分组是**同一个决定的两种写法**，所以放在同一处，由
//! [`UsageLang`] 一次选定。
//!
//! 英文单位表里没有 `G`：`B`（billion，十亿）才是计数场合的写法，`G` 是字节
//! 那套的。

use crate::server::wire::UsageLang;

/// 缩略写法：`12.3M` / `1.2万`。给人扫一眼用的。
pub fn compact(n: u64, lang: UsageLang) -> String {
    let units: &[(u64, &str)] = match lang {
        UsageLang::En => &[
            (1_000_000_000_000, "T"),
            (1_000_000_000, "B"),
            (1_000_000, "M"),
            (1_000, "K"),
        ],
        // 中文四位一进：万、亿、兆、京（亿×亿 = 京）。
        UsageLang::Zh => &[
            (10_000_000_000_000_000, "京"),
            (1_000_000_000_000, "兆"),
            (100_000_000, "亿"),
            (10_000, "万"),
        ],
    };
    for (scale, suffix) in units {
        if n >= *scale {
            // 一位小数：这一栏是给人扫的，"1.2M" 与 "1.23M" 的信息差抵不上
            // 多占的那两列。
            return format!("{:.1}{suffix}", n as f64 / *scale as f64);
        }
    }
    n.to_string()
}

/// 确切数额：**不缩写**，按口径分组。要对数的时候看它。
///
/// 英文三位一断（`2,440,000`），中文四位一断（`244,0000`）——分组宽度与
/// [`compact`] 的进位基数一致，两行说的才是同一件事。
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

/// 金额，带货币符号。
///
/// **精度跟着数量级走**：两位小数对一分钱以上的账精确，对一厘以下的账只会印成
/// `¥0.00`——用户看到的是"这个组件坏了"。所以小额改成两位有效数字，保证"花了
/// 多少"永远有个非零的数字可看。
pub fn money(v: f64, symbol: &str) -> String {
    if !v.is_finite() || v <= 0.0 {
        return format!("{symbol}0.00");
    }
    if v >= 0.01 {
        return format!("{symbol}{v:.2}");
    }
    // 0.000018 -> 6 位（两位有效数字）：log10 取整到下一个数量级再加一位。
    let digits = (-(v.log10().floor()) as i32 + 1).clamp(2, 8) as usize;
    format!("{symbol}{v:.digits$}")
}

/// 金额，**尽量在 `max_w` 列内展开**——空间够就写全，不够就收窄，收到两位
/// 小数为止。
///
/// 和 [`money`] 是一对：那个按**数量级**决定精度（一厘以下的账也得看得见），
/// 这个再按**列宽**收一层（窄终端放不下那么长的数）。
///
/// 收窄的顺序是有讲究的：
///
/// 1. 写全（`¥0.000018`）——有地方就一个字符都不省；
/// 2. 两位小数（`¥0.00`）——列宽只够常规账；
/// 3. 整数（`¥0`）——连小数点都放不下时，宁可丢小数也不丢数字。
///
/// 不会再往下砍成残缺的数（`¥0.00…` 那种），那比不显示更糟：它看起来是个数。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_two_decimals_until_it_would_round_to_nothing() {
        assert_eq!(money(2.77, "$"), "$2.77");
        assert_eq!(money(1.5, "¥"), "¥1.50");
        assert_eq!(money(0.5, "¥"), "¥0.50");
        // 一分钱以下：印成 0.00 等于告诉用户"没花钱"，那是假的。
        assert_eq!(money(0.02, "¥"), "¥0.02");
        assert_eq!(money(0.009, "¥"), "¥0.0090");
        assert_eq!(money(0.000018, "¥"), "¥0.000018");
        assert_eq!(money(0.0, "¥"), "¥0.00");
    }

    #[test]
    fn money_shrinks_to_fit_the_columns_it_has() {
        // 空间够：一个字符都不省。
        assert_eq!(money_within(0.000018, "¥", 11), "¥0.000018");
        assert_eq!(money_within(2.77, "¥", 11), "¥2.77");
        // 中等：还是写全（9 字符正好放下）。
        assert_eq!(money_within(0.000018, "¥", 9), "¥0.000018");
        // 窄：退到两位小数。
        assert_eq!(money_within(0.000018, "¥", 8), "¥0.00");
        assert_eq!(money_within(0.000018, "¥", 5), "¥0.00");
        // 更窄：连小数点都不要了，但数字还得在。
        assert_eq!(money_within(2.77, "¥", 3), "¥3");
        // 绝不产出残缺写法（`¥0.00…` 那种看着像个数）。
        for w in 3..=12 {
            let out = money_within(0.000018, "¥", w);
            assert!(!out.contains('…'), "@{w}: {out}");
        }
    }

    #[test]
    fn english_uses_three_digit_units() {
        assert_eq!(compact(0, UsageLang::En), "0");
        assert_eq!(compact(590, UsageLang::En), "590");
        assert_eq!(compact(1_000, UsageLang::En), "1.0K");
        assert_eq!(compact(1_234, UsageLang::En), "1.2K");
        assert_eq!(compact(12_345_678, UsageLang::En), "12.3M");
        assert_eq!(compact(2_400_000_000, UsageLang::En), "2.4B");
        assert_eq!(compact(3_000_000_000_000, UsageLang::En), "3.0T");
    }

    #[test]
    fn chinese_uses_four_digit_units() {
        // 中文读数四位一块：万、亿、兆、京。
        assert_eq!(compact(9999, UsageLang::Zh), "9999");
        assert_eq!(compact(10_000, UsageLang::Zh), "1.0万");
        assert_eq!(compact(12_345, UsageLang::Zh), "1.2万");
        assert_eq!(compact(100_000_000, UsageLang::Zh), "1.0亿");
        assert_eq!(compact(1_000_000_000_000, UsageLang::Zh), "1.0兆");
        assert_eq!(compact(10_000_000_000_000_000, UsageLang::Zh), "1.0京");
    }

    #[test]
    fn the_exact_number_groups_by_the_same_base_as_the_short_one() {
        assert_eq!(exact(2_440_000, UsageLang::En), "2,440,000");
        assert_eq!(exact(244_0000, UsageLang::Zh), "244,0000");
        assert_eq!(exact(1_2345_6789, UsageLang::Zh), "1,2345,6789");
        assert_eq!(exact(1234, UsageLang::Zh), "1234", "不到一万不分组");
        assert_eq!(exact(123, UsageLang::En), "123");
    }
}
