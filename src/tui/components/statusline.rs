//! Statusline — exactly per the user's spec:
//!
//! +--[π > [M]{Model}(accent) > [D]Utility/MyPi > @{branch} > {cost}]---{ctx%}-----:1M | <[{session}]--+
//!    \_________pure-black capsules, accent text, gray pi_________/  \_acc_/_plain_/\acc/\black capsule/borders acc
//!
//! - cost: session total (not per-turn), with the currency symbol (models.yml currency)
//! - ctx gauge: `>`---{pct}---` accent dashes on both sides, no background; denominator `:1M`
//! - fixed `+--` / `--+` at both ends; the pure-black background applies only inside [] capsules

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::tui::theme::Palette;

// Statusline data.
pub struct StatusInfo<'a> {
    pub model_name: &'a str,
    pub cwd: &'a str,
    pub ctx_tokens: u64,
    pub ctx_limit: u64,
    // Session cumulative cost (in the primary currency).
    pub total_cost: f64,
    pub currency_symbol: &'a str,
    pub session_name: &'a str,
    // False when the model has no pricing; the cost segment is hidden.
    pub show_cost: bool,
    // git status: branch name + changed file counts (None = outside a repo).
    pub git: Option<&'a crate::git::GitStatus>,
}

// Token count: 1_000_000 -> "1M", 28_000 -> "28K", 590 -> "590".
pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = n as f64 / 1_000_000.0;
        if m.fract() == 0.0 {
            format!("{}M", m as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if n >= 1000 {
        let k = n as f64 / 1000.0;
        if k.fract() == 0.0 {
            format!("{}K", k as u64)
        } else {
            format!("{k:.1}K")
        }
    } else {
        format!("{n}")
    }
}

// Directory: at most one parent level. /home/Arisha/Utility/MyPi -> Utility/MyPi
pub fn short_cwd(cwd: &str) -> String {
    let parts: Vec<&str> = cwd
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    match parts.len() {
        0 => "/".into(),
        1 => format!("/{}", parts[0]),
        _ => format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]),
    }
}

// Money: fixed two decimals, with the currency symbol.
pub fn fmt_money(v: f64, symbol: &str) -> String {
    format!("{symbol}{v:.2}")
}

// The statusline: **one row**, forming the top edge of the input container.
//
// `+--pi > model > path > @branch > cost >----ctx 8%-----:1M | <[session]--+`
//
// Colors:
// - `+--` / `--+` at the ends: accent green
// - `π`: gray, the only permitted gray
// - `[M]` / `[D]`：accent
// - model name / path / branch: accent
// - cost: gold
// - ctx: **transparent background**, used part accent, unused part plain; shows only the percentage (e.g. `8%`)
// - capsule background: black (from the theme)
//
// `spinner`: animation frame while waiting (e.g. `|`/`/`/`-`/`\\`); `None` means idle, showing pi.
pub fn render(info: &StatusInfo, p: &Palette, width: u16, spinner: Option<char>) -> Line<'static> {
    let w = width as usize;

    // ---- left black capsules: pi/spinner > [M]model > [D]path > @branch > cost ----
    // The [M]/[D] markers are accent too, keeping the whole capsule one color.
    let mut crumbs: Vec<Span> = vec![
        p.on_black(" "), // one black cell before the glyph, for visual balance
        p.symbol(spinner),
    ];
    crumbs.push(p.on_black(" > "));
    crumbs.push(p.mark("[M]"));
    crumbs.push(p.pill(info.model_name, p.accent));
    crumbs.push(p.on_black(" > "));
    crumbs.push(p.mark("[D]"));
    crumbs.push(p.pill(short_cwd(info.cwd), p.accent));
    if let Some(g) = info.git {
        // Position: between `[D]path` and the cost
        if let Some(b) = &g.branch {
            crumbs.push(p.on_black(" > @"));
            crumbs.push(p.pill(b, p.accent));
        }
        // ?N = files needing add; +N = staged files. Counts files, never lines.
        if g.unstaged > 0 {
            crumbs.push(p.on_black(" ?"));
            crumbs.push(p.pill(g.unstaged.to_string(), p.gold));
        }
        if g.staged > 0 {
            crumbs.push(p.on_black(" +"));
            crumbs.push(p.pill(g.staged.to_string(), p.gold));
        }
    }
    if info.show_cost {
        crumbs.push(p.on_black(" > "));
        crumbs.push(p.gold_on_black(fmt_money(info.total_cost, info.currency_symbol)));
    }
    crumbs.push(p.on_black(" "));

    // ---- live ctx percentage: the number only, e.g. `8%` ----
    let denom = fmt_tokens(info.ctx_limit);
    let pct = if info.ctx_limit > 0 {
        (info.ctx_tokens as f64 / info.ctx_limit as f64 * 100.0).round() as u64
    } else {
        0
    };
    let pct_txt = format!("{pct}%");
    let sess_txt = info.session_name.to_string();

    // head: `+--` + capsules + `>`; tail: `:1M | session--+`
    let head = |crumbs: &[Span<'static>]| -> Vec<Span<'static>> {
        let mut v: Vec<Span<'static>> = Vec::new();
        v.push(p.accent_span("+"));
        v.push(adash(p, 2)); // `+--`: one more dash than the bottom edge
        v.extend(crumbs.iter().cloned());
        v.push(p.accent_span(">")); // gauge start
        v
    };
    let tail: Vec<Span<'static>> = vec![
        p.accent_span(format!(":{denom}")),
        p.accent_span(" | "),
        p.accent_span("<"),                 // left chevron of the session segment
        p.on_black(" "),                    // black cell after the chevron (cosmetic)
        p.pill(sess_txt.clone(), p.accent), // black background only, the name itself
        p.on_black(" "),                    // black cell on the right
        adash(p, 2),                        // `--`
        p.accent_span("+"),                 // fixed right end
    ];
    let tail_w: usize = tail.iter().map(|s| s.content.width()).sum();

    let mut spans = head(&crumbs);
    let mut head_w: usize = spans.iter().map(|s| s.content.width()).sum();

    // Narrow screens: shrink capsules, then zero out the gauge
    if head_w + tail_w + pct_txt.width() > w {
        crumbs = truncate_crumbs(p, crumbs, 8);
        spans = head(&crumbs);
        head_w = spans.iter().map(|s| s.content.width()).sum();
    }
    let dash_w = w.saturating_sub(head_w + tail_w + pct_txt.width());
    // Used cells computed live from the percentage; the first cell right of `>` is always accent (purely cosmetic)
    let used = ((pct as usize) * dash_w / 100).min(dash_w);
    let lead = usize::from(dash_w > 0);
    let used_rest = used.saturating_sub(lead);
    let plain_rest = dash_w.saturating_sub(lead + used_rest);

    spans.push(adash(p, lead)); // first cell after `>`: always accent
    spans.push(adash(p, used_rest)); // used part: accent
    spans.push(p.accent_span(&pct_txt)); // 8%
    spans.push(p.plain("-".repeat(plain_rest))); // unused: transparent background, plain
    spans.extend(tail);

    Line::from(fit_width(spans, w))
}

// Fallback: hard-truncate by display width when still too wide; pad with plain spaces when short.
fn fit_width(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used <= w {
        let mut out = spans;
        out.push(Span::styled(" ".repeat(w - used), Style::new()));
        return out;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut acc = 0usize;
    for s in spans {
        let sw = s.content.width();
        if acc + sw <= w {
            acc += sw;
            out.push(s);
        } else {
            let mut kept = String::new();
            for ch in s.content.chars() {
                let cw = ch.to_string().width();
                if acc + cw > w {
                    break;
                }
                kept.push(ch);
                acc += cw;
            }
            out.push(Span::styled(kept, s.style));
            break;
        }
    }
    out
}

// On narrow terminals truncate the breadcrumb to the budget, appending `> ...`.
fn truncate_crumbs(p: &Palette, crumbs: Vec<Span<'static>>, budget: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span> = Vec::new();
    let mut used = 1; // π
    for s in crumbs {
        let len = s.content.width();
        if used + len > budget.saturating_sub(3) {
            break;
        }
        used += len;
        out.push(s);
    }
    out.push(p.pill(" > …", p.accent));
    out
}

fn adash(p: &Palette, n: usize) -> Span<'static> {
    p.accent_span("-".repeat(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn token_fmt() {
        assert_eq!(fmt_tokens(1_000_000), "1M");
        assert_eq!(fmt_tokens(28_000), "28K");
        assert_eq!(fmt_tokens(590), "590");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn cwd_shorten() {
        assert_eq!(short_cwd("/home/Arisha/Utility/MyPi"), "Utility/MyPi");
        assert_eq!(short_cwd("/home"), "/home");
    }

    #[test]
    fn money_fmt() {
        assert_eq!(fmt_money(0.0004, "$"), "$0.00");
        assert_eq!(fmt_money(2.77, "$"), "$2.77");
        assert_eq!(fmt_money(0.001, "¥"), "¥0.00");
        assert_eq!(fmt_money(1.5, "¥"), "¥1.50");
    }

    // The statusline fits exactly one row of width, symmetric `+--` / `--+` at the ends.
    #[test]
    fn statusline_single_line_fits_and_symmetric() {
        let info = StatusInfo {
            model_name: "global:model-z",
            cwd: "/home/Arisha/Utility/MyPi",
            git: Some(&crate::git::GitStatus {
                branch: Some("main".into()),
                unstaged: 0,
                staged: 0,
            }),
            ctx_tokens: 590,
            ctx_limit: 1_000_000,
            total_cost: 0.0042,
            currency_symbol: "$",
            session_name: "GPT5.6L(LC)",
            show_cost: true,
        };
        let p = Palette::default();
        let text = |l: &Line| -> String { l.spans.iter().map(|s| s.content.to_string()).collect() };
        for width in [60u16, 100, 160, 220] {
            let line = render(&info, &p, width, None);
            let t = text(&line);
            let tw: usize = line.spans.iter().map(|s| s.content.width()).sum();
            assert_eq!(tw, width as usize, "width @{width}: {t}");
            assert!(t.starts_with("+--"), "@{width}: {t}");
            assert!(t.ends_with("--+"), "@{width}: {t}");
        }
    }

    // Colors: ends/[M]/[D]/model name accent, pi gray, cost gold,
    // ctx unused part **transparent background** (no bg set), showing `N%` only.
    #[test]
    fn colors_match_spec() {
        let info = StatusInfo {
            model_name: "global:model-z",
            cwd: "/home/Arisha/Utility/MyPi",
            git: Some(&crate::git::GitStatus {
                branch: Some("main".into()),
                unstaged: 0,
                staged: 0,
            }),
            ctx_tokens: 80_000,
            ctx_limit: 1_000_000,
            total_cost: 1.2345,
            currency_symbol: "¥",
            session_name: "GPT5.6L(LC)",
            show_cost: true,
        };
        let p = Palette::default();
        let line = render(&info, &p, 120, None);
        let find = |needle: &str| -> Option<Style> {
            line.spans
                .iter()
                .find(|s| s.content == needle)
                .map(|s| s.style)
        };
        // Fixed end lines
        assert_eq!(find("+").unwrap().fg, Some(p.accent));
        // pi: gray on black
        let pi = find("π").unwrap();
        assert_eq!(pi.fg, Some(p.muted));
        assert_eq!(pi.bg, Some(p.black));
        // [M] / [D]：accent
        assert_eq!(find("[M]").unwrap().fg, Some(p.accent));
        assert_eq!(find("[D]").unwrap().fg, Some(p.accent));
        // Model name / path: accent
        assert_eq!(find("global:model-z").unwrap().fg, Some(p.accent));
        assert_eq!(find("Utility/MyPi").unwrap().fg, Some(p.accent));
        // Cost: gold
        assert_eq!(find("¥1.23").unwrap().fg, Some(p.gold));
        // Percentage shows `8%` only, no "ctx" prefix
        assert!(line.spans.iter().any(|s| s.content == "8%"));
        assert!(!line.spans.iter().any(|s| s.content.contains("ctx")));
        // Unused dashes: transparent background (no bg set)
        let rest = line
            .spans
            .iter()
            .find(|s| {
                !s.content.is_empty() && s.content.chars().all(|c| c == '-') && s.style.fg.is_none()
            })
            .expect("there should be a plain transparent dash run");
        assert_eq!(
            rest.style.bg, None,
            "unused ctx part must have no background"
        );
    }

    // The accent comes from config, not hardcoded: switching theme colors changes rendering.
    #[test]
    fn accent_is_configurable() {
        let info = StatusInfo {
            model_name: "m",
            cwd: "/a/b",
            git: None,
            ctx_tokens: 0,
            ctx_limit: 100,
            total_cost: 0.0,
            currency_symbol: "$",
            session_name: "s",
            show_cost: false,
        };
        let p = Palette {
            accent: Color::Magenta,
            gold: Color::Cyan,
            black: Color::Rgb(1, 2, 3),
            muted: Color::Blue,
        };
        let line = render(&info, &p, 80, None);
        let plus = line.spans.iter().find(|s| s.content == "+").unwrap();
        assert_eq!(plus.style.fg, Some(Color::Magenta));
        let pi = line.spans.iter().find(|s| s.content == "π").unwrap();
        assert_eq!(pi.style.bg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(pi.style.fg, Some(Color::Blue));
    }

    // Spinner: while waiting the glyph becomes an accent-colored frame; idle shows a gray pi.
    #[test]
    fn spinner_replaces_pi_while_waiting() {
        let info = StatusInfo {
            model_name: "m",
            cwd: "/a/b",
            git: None,
            ctx_tokens: 0,
            ctx_limit: 100,
            total_cost: 0.0,
            currency_symbol: "$",
            session_name: "s",
            show_cost: false,
        };
        let p = Palette::default();
        // Idle: gray pi
        let idle = render(&info, &p, 80, None);
        let pi = idle.spans.iter().find(|s| s.content == "π").unwrap();
        assert_eq!(pi.style.fg, Some(p.muted));
        assert!(idle.spans.iter().all(|s| s.content != "|"));

        // Waiting: the glyph becomes an accent spinner frame, no pi
        for (i, ch) in ['|', '/', '-', '\\'].iter().enumerate() {
            let busy = render(&info, &p, 80, Some(*ch));
            assert!(
                busy.spans.iter().all(|s| s.content != "π"),
                "pi must be gone while waiting (frame {i})"
            );
            let sp = busy
                .spans
                .iter()
                .find(|s| s.content == ch.to_string())
                .unwrap();
            assert_eq!(sp.style.fg, Some(p.accent), "spinner frames must be accent");
        }
    }

    // git segment: after [D]path, before the cost; ?N/+N in gold.
    #[test]
    fn git_segment_renders_between_path_and_cost() {
        let git = crate::git::GitStatus {
            branch: Some("main".into()),
            unstaged: 9,
            staged: 2,
        };
        let info = StatusInfo {
            model_name: "m",
            cwd: "/a/b",
            ctx_tokens: 0,
            ctx_limit: 100,
            total_cost: 1.5,
            currency_symbol: "¥",
            session_name: "s",
            show_cost: true,
            git: Some(&git),
        };
        let p = Palette::default();
        let line = render(&info, &p, 120, None);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("> @main"), "{text}");
        assert!(text.contains(" ?9"), "{text}");
        assert!(text.contains(" +2"), "{text}");
        // Order: path < @branch < ?N < +N < cost
        let i_path = text.find("> [D]a/b").unwrap();
        let i_br = text.find("@main").unwrap();
        let i_q = text.find("?9").unwrap();
        let i_p = text.find("+2").unwrap();
        let i_money = text.find("¥1.50").unwrap();
        assert!(
            i_path < i_br && i_br < i_q && i_q < i_p && i_p < i_money,
            "{text}"
        );
        // Counts in gold
        let q = line.spans.iter().find(|s| s.content == "9").unwrap();
        assert_eq!(q.style.fg, Some(p.gold));
    }

    // Clean repo: no ?/+ segments, branch only.
    #[test]
    fn clean_git_hides_counts() {
        let git = crate::git::GitStatus {
            branch: Some("dev".into()),
            unstaged: 0,
            staged: 0,
        };
        let info = StatusInfo {
            model_name: "m",
            cwd: "/a/b",
            ctx_tokens: 0,
            ctx_limit: 100,
            total_cost: 0.0,
            currency_symbol: "$",
            session_name: "s",
            show_cost: false,
            git: Some(&git),
        };
        let p = Palette::default();
        let text: String = render(&info, &p, 120, None)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.contains("@dev"), "{text}");
        assert!(!text.contains(" ?"), "no ?N expected: {text}");
        assert!(!text.contains(" +"), "no +N expected: {text}");
    }
}
