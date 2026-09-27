//! `/usage` 面板 —— 保留区的住户之一。
//!
//! 数据由服务端算好（[`ServerMsg::Usage`](crate::server::wire::ServerMsg)）：
//! 谁问过哪些模型、各花了多少 token、拿当前价格表折成多少钱。前端只负责排版，
//! 一行不算账——算账要价格表，价格表在服务端的配置里。
//!
//! 版式固定三行头 + 一行一个模型：
//!
//! ```text
//!  Total Token: 2.4M, exactly: 2,440,000
//!  Cache Hit Rate: 83.33%
//!
//!  Model              Total      Input      Output     CacheRead  CacheWrite
//! >GLM5.3F(WB)        ¥4.62      ¥1.20      ¥3.40      ¥0.02      ¥0.00
//!  DSV4.1F(WB)        ¥1.74      ¥0.50      ¥1.20      ¥0.04      ¥0.00
//! ```
//!
//! 前两行是**整个会话的合计**（不分模型），后面才是分模型的表。没有竖线的
//! 规矩表：列宽固定、右对齐，靠位置对齐而不是靠框线。
//!
//! 面板**不进历史**：它不是谁说过的话，不落盘、不参与上下文，Esc 一关就没了。
//! 这也正是它住在保留区而不是转录里的原因。

use ratatui::text::{Line, Span};

use crate::server::ai::config::UsageAlign;
use crate::server::store::ModelUsage;
use crate::server::wire::UsageLang;
use crate::tui::num;
use crate::tui::theme::Palette;
use crate::tui::zone::main::input::semantics::Action;
use crate::tui::zone::main::reserved::area::Claim;

/// 面板最多占几行（够放下二十来个模型；再多会被终端高度裁）。
const MAX_ROWS: usize = 24;
/// 表格最左边那个选中标记占的列数（`>` 或空格）。
const MARKER_W: usize = 1;

/// 版式档位：`(名字列宽, 金额列宽, 表头名)`，宽 → 窄。
///
/// **金额永远是五列**（Total / Input / Output / CacheRead / CacheWrite）：
/// 用户要的就是"一个模型一行看全"，砍列就把这件事毁了。窄终端下能压的只有
/// 列宽和表头的写法——列本身不许少。
///
/// 表头名跟着列宽一起缩：`CacheWrite` 要 10 列，硬塞进 8 列会印成
/// `CacheWri…` 并且和左邻那格粘成一片。缩写是诚实的做法，前两档给全称。
///
/// 每档的实宽 = `name + 5 * money`：73 / 58 / 49 / 42 / 35 列（再加上标记那
/// 一格，放得下才画）。比最窄一档还窄的终端就让行尾被裁（`clip_line` 兜底），
/// 不会再切档。
const LAYOUTS: [(usize, usize, [&str; 5]); 5] = [
    (
        18,
        11,
        ["Total", "Input", "Output", "CacheRead", "CacheWrite"],
    ),
    (13, 9, ["Total", "Input", "Output", "CRead", "CWrite"]),
    (9, 8, ["Total", "In", "Out", "CRead", "CWrite"]),
    (7, 7, ["Tot", "In", "Out", "CRe", "CWr"]),
    // 四十列也放得下的兜底：实宽 36。再窄就真的只能裁了。
    (5, 6, ["Tot", "In", "Out", "CR", "CW"]),
];

#[derive(Debug, Default)]
pub struct UsagePanel {
    lang: UsageLang,
    groups: Vec<ModelUsage>,
    /// 上下键选中的那一行（表格里的下标）。
    selected: usize,
    /// 整块贴哪边（`config.yaml` → `usage.align`）。只影响水平留白。
    align: UsageAlign,
    open: bool,
}

impl UsagePanel {
    /// 打开（或替换）面板。`/usage` 再敲一次就是重新算一遍并重开。
    pub fn show(&mut self, lang: UsageLang, groups: Vec<ModelUsage>) {
        self.lang = lang;
        self.groups = groups;
        self.selected = 0;
        self.open = true;
    }

    /// 整块的水平位置（装配时从配置来一次）。
    pub fn set_align(&mut self, align: UsageAlign) {
        self.align = align;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 三行头（总量、命中率、表头）+ 一行一个模型。空会话时表里没有行，换成
    /// 一句"还没跑过"——那件事本身也要占一行说，不然用户看到的是空白。
    fn rows(&self) -> usize {
        let body = self.groups.len().max(1);
        (3 + body).min(MAX_ROWS)
    }

    /// 上下键移动选中行，**绕回**：到底再按一下回到第一行，比卡在两头顺手。
    fn step(&mut self, delta: isize) {
        let n = self.groups.len();
        if n == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(n as isize) as usize;
    }
}

impl Claim for UsagePanel {
    fn id(&self) -> &'static str {
        "usage"
    }

    fn want(&self) -> Option<usize> {
        self.open.then(|| self.rows())
    }

    fn max(&self) -> usize {
        MAX_ROWS
    }

    fn accepts(&self) -> &'static [Action] {
        // 上/下 = 选模型，Tab = 换数字口径。上下拿走之后面板开着时编辑器就
        // 没有上下键了——这是浮层该有的样子，它在上层。
        &[Action::DismissPanel, Action::Up, Action::Down, Action::Complete]
    }

    fn on_key(&mut self, action: &Action) -> bool {
        match action {
            Action::DismissPanel => {
                self.close();
                true
            }
            Action::Up => {
                self.step(-1);
                true
            }
            Action::Down => {
                self.step(1);
                true
            }
            Action::Complete => {
                // Tab 换口径：中英文两套不只是单位名，进位基数也跟着换。
                self.lang = match self.lang {
                    UsageLang::En => UsageLang::Zh,
                    UsageLang::Zh => UsageLang::En,
                };
                true
            }
            _ => false,
        }
    }

    fn draw(&self, term_w: u16, rows: usize, p: &Palette) -> Vec<Line<'static>> {
        let width = usize::from(term_w);
        let (name_w, money_w, heads) = fit(width);
        // 整块的位置由配置定（默认居中）。
        //
        // 表格的列宽**只由名字和五栏决定**：`>` 是额外的一列，落在内容左边
        // 或整个不画（accent 色已经把"选中"说清楚了），它绝不挤占列宽——那
        // 就成了插足。所以 `Total Token` / `Cache Hit Rate` / `Model` 三行的
        // 首字符、以及每个模型名的首字符，永远在同一条竖线上。
        let block_w = name_w + money_w * 5;
        let marker = MARKER_W <= width.saturating_sub(block_w);
        let full_w = if marker { block_w + MARKER_W } else { block_w };
        let slack = width.saturating_sub(full_w);
        let lead = " ".repeat(match self.align {
            UsageAlign::Left => 0,
            UsageAlign::Center => slack / 2,
            UsageAlign::Right => slack,
        });
        // 头部与表头：标记那一格留空（有它的话）。
        let pad = format!("{lead}{}", if marker { " " } else { "" });
        let mut out: Vec<Line<'static>> = Vec::with_capacity(rows);
        let total = Total::of(&self.groups);

        out.push(Line::from(vec![
            Span::raw(pad.clone()),
            p.muted(match self.lang {
                UsageLang::En => "Total Tokens: ",
                UsageLang::Zh => "总词元：",
            }),
            p.accent_span(num::compact(total.tokens, self.lang)),
            p.muted(match self.lang {
                UsageLang::En => ", exactly: ",
                UsageLang::Zh => "，确切数额：",
            }),
            Span::raw(num::exact(total.tokens, self.lang)),
        ]));
        out.push(Line::from(vec![
            Span::raw(pad.clone()),
            p.muted(match self.lang {
                UsageLang::En => "Cache Hit Rate: ",
                UsageLang::Zh => "缓存命中率：",
            }),
            // 分母是**输入**：缓存读是输入的一部分，除上总 token 只会把它摊薄成
            // 一个偏小、看不懂的数。
            Span::raw(match total.prompt {
                0 => "-".to_string(),
                prompt => format!("{:.2}%", total.cached as f64 / prompt as f64 * 100.0),
            }),
        ]));
        out.push(Line::from(vec![
            Span::raw(pad.clone()),
            p.muted(self.header(name_w, money_w, &heads)),
        ]));

        if self.groups.is_empty() {
            out.push(Line::from(vec![
                Span::raw(pad),
                p.muted(match self.lang {
                    UsageLang::En => "(no rounds yet)",
                    UsageLang::Zh => "（还没跑过任何回合）",
                }),
            ]));
        } else {
            for (i, g) in self.groups.iter().enumerate() {
                out.push(self.model_row(
                    g,
                    i == self.selected,
                    name_w,
                    money_w,
                    &lead,
                    marker,
                    p,
                ));
            }
        }

        // 收口统一在这里：**每一行**都要裁（表头也会长过窄终端），行数还要
        // 正好是仲裁发下来的那个——保留区的高度由 terminal 说了算。
        for line in &mut out {
            clip_line(line, width);
        }
        out.truncate(rows);
        out.resize_with(rows, || Line::from(""));
        out
    }
}

impl UsagePanel {
    /// 表头。第一列给名字，后面四列是账单的四栏——模型行第一格必须是名字，
    /// 所以 `Model` 这一列不是可有可无的。
    fn header(&self, name_w: usize, money_w: usize, heads: &[&str; 5]) -> String {
        let mut head = format!("{:<name_w$}", "Model");
        for col in heads {
            let cell = clip(col, money_w);
            head.push_str(&format!("{cell:>money_w$}"));
        }
        head
    }

    /// 一个模型一行：名字 + 合计 + 四栏。选中那行整行上色并带 `>` 标记。
    #[allow(clippy::too_many_arguments)] // 版式参数一条一条传，比塞个结构体清楚
    fn model_row(
        &self,
        g: &ModelUsage,
        selected: bool,
        name_w: usize,
        money_w: usize,
        lead: &str,
        marker: bool,
        p: &Palette,
    ) -> Line<'static> {
        // 标记是额外的一格；它没有被画出来的余量时就不画（accent 色已经指出
        // 哪一行是选中的），名字的位置不受影响。
        let flag = match (marker, selected) {
            (true, true) => ">",
            (true, false) => " ",
            (false, _) => "",
        };
        let name = clip(&g.name, name_w);
        let Some(cost) = g.cost else {
            // 认不出价格表（配置里删了/改名了）：写不出来就说写不出来，
            // 拿 0.00 顶上等于告诉用户这个模型不要钱。
            return Line::from(vec![
                Span::raw(format!("{lead}{flag}{name:<name_w$} ")),
                p.muted(match self.lang {
                    UsageLang::En => "(no price sheet)",
                    UsageLang::Zh => "（没有价格表）",
                }),
            ]);
        };
        let symbol = g.currency.as_deref().unwrap_or("");
        let total = cost.input + cost.output + cost.cache_read + cost.cache_write;
        let mut text = format!("{lead}{flag}{name:<name_w$}");
        for v in [total, cost.input, cost.output, cost.cache_read, cost.cache_write] {
            // 金额自己按列宽选精度：宽了写全（`¥0.000018`），窄了收到两位
            // （`¥0.00`）。裁出来的残数（`¥0.00…`）看着像个数，比不显示更糟。
            let cell = num::money_within(v, symbol, money_w);
            text.push_str(&format!("{cell:>money_w$}"));
        }
        if selected {
            return Line::from(p.accent_span(text));
        }
        Line::from(Span::raw(text))
    }
}

/// 整个会话的合计（头部两行要的是它，不是某一个模型）。
struct Total {
    tokens: u64,
    prompt: u64,
    cached: u64,
}

impl Total {
    fn of(groups: &[ModelUsage]) -> Self {
        let mut t = Self {
            tokens: 0,
            prompt: 0,
            cached: 0,
        };
        for g in groups {
            t.tokens += g.prompt_tokens + g.completion_tokens;
            t.prompt += g.prompt_tokens;
            t.cached += g.cached_tokens;
        }
        t
    }
}

/// 按可用宽度挑一档版式：能放下最宽的就用最宽的。
fn fit(width: usize) -> (usize, usize, [&'static str; 5]) {
    for (name_w, money_w, heads) in LAYOUTS {
        // 判据是**表格**的宽度：标记那一格是额外的，它放不下不该拖着表格一起
        // 降档——那样用户会为一格标记损失两列宽度。
        if name_w + money_w * 5 <= width {
            return (name_w, money_w, heads);
        }
    }
    // 比最窄一档还窄：就用最窄的，行尾由 `clip_line` 裁。
    LAYOUTS[LAYOUTS.len() - 1]
}

/// 一行的**显示宽度**（中文按两列算）。`len()` 是字节数，中文行会大出一倍。
/// 只有测试要按宽度看齐，生产路径的裁剪自己算。
#[cfg(test)]
fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}

/// 截到 `w` 个字符（按字符算，不是字节——名字里有中文）。
fn clip(s: &str, w: usize) -> String {
    let mut out: String = s.chars().take(w).collect();
    if s.chars().count() > w {
        out.pop();
        out.push('…');
    }
    out
}

/// 按显示宽度裁掉行尾多余的部分：保留区的宽度由终端说了算，一行不允许越界。
///
/// 装不下的那一格是**截断**而不是丢掉——整行都由单个 span 拼出来的地方（表头
/// 就是这样），丢掉等于这一行凭空消失，屏幕上只剩空行。
fn clip_line(line: &mut Line<'static>, width: usize) {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let mut used = 0usize;
    let mut cut = None;
    for (i, span) in line.spans.iter().enumerate() {
        let w = span.content.width();
        if used + w <= width {
            used += w;
            continue;
        }
        cut = Some(i);
        break;
    }
    let Some(i) = cut else { return };
    let room = width.saturating_sub(used);
    let mut keep = String::new();
    let mut w = 0usize;
    for ch in line.spans[i].content.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > room {
            break;
        }
        w += cw;
        keep.push(ch);
    }
    line.spans[i].content = keep.into();
    line.spans.truncate(i + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::store::UsageCost;

    fn group(name: &str, prompt: u64, completion: u64, cached: u64) -> ModelUsage {
        ModelUsage {
            name: name.into(),
            id: name.into(),
            provider: Some("wb".into()),
            rounds: 3,
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: cached,
            cost: Some(UsageCost {
                input: 1.20,
                output: 3.40,
                cache_read: 0.02,
                cache_write: 0.0,
            }),
            currency: Some("¥".into()),
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn lines(p: &UsagePanel) -> Vec<String> {
        let theme = Palette::current();
        let rows = usize::from(p.want().unwrap());
        p.draw(200, rows, &theme).iter().map(text).collect()
    }

    fn panel(lang: UsageLang, groups: Vec<ModelUsage>) -> UsagePanel {
        let mut p = UsagePanel::default();
        p.show(lang, groups);
        p
    }

    #[test]
    fn a_closed_panel_claims_nothing() {
        let p = UsagePanel::default();
        assert_eq!(p.want(), None, "没开的时候不能占保留区的行高");
    }

    #[test]
    fn the_head_two_rows_are_the_whole_session_not_one_model() {
        // 三个模型各 1M 输入：头部必须是 3M 这个合计，不是其中任何一个。
        let p = panel(
            UsageLang::En,
            vec![
                group("A", 1_000_000, 0, 900_000),
                group("B", 1_000_000, 0, 800_000),
                group("C", 1_000_000, 0, 700_000),
            ],
        );
        let out = lines(&p);
        assert!(out[0].contains("3.0M"), "总量是三个模型之和：{}", out[0]);
        assert!(out[0].contains("3,000,000"), "确切值：{}", out[0]);
        // 命中率 = 缓存读总量 / 输入总量 = 240万/300万 = 80.00%
        assert!(out[1].contains("80.00%"), "{}", out[1]);
    }

    #[test]
    fn english_and_chinese_differ_in_base_not_just_in_labels() {
        let en = lines(&panel(UsageLang::En, vec![group("A", 24_400_000, 0, 0)]));
        assert!(en[0].contains("24.4M"), "{}", en[0]);
        assert!(en[0].contains("24,400,000"), "{}", en[0]);

        let zh = lines(&panel(UsageLang::Zh, vec![group("A", 24_400_000, 0, 0)]));
        assert!(zh[0].contains("2440.0万"), "中文四位一进：{}", zh[0]);
        assert!(zh[0].contains("2440,0000"), "确切值四位分组：{}", zh[0]);
        assert!(zh[1].contains("缓存命中率"), "{}", zh[1]);
    }

    #[test]
    fn the_table_has_a_model_column_and_four_money_columns() {
        let p = panel(UsageLang::En, vec![group("GLM5.3F(WB)", 1000, 10, 0)]);
        let out = lines(&p);
        let head = &out[2];
        for col in ["Model", "Total", "Input", "Output", "CacheRead", "CacheWrite"] {
            assert!(head.contains(col), "缺 {col} 列：{head}");
        }
        let row = &out[3];
        assert!(row.contains("GLM5.3F(WB)"), "{row}");
        assert!(row.contains("¥4.62"), "合计：{row}");
        assert!(row.contains("¥1.20"), "{row}");
    }

    #[test]
    fn up_and_down_move_the_highlight_and_wrap_around() {
        let mut p = panel(
            UsageLang::En,
            vec![group("A", 1, 0, 0), group("B", 1, 0, 0), group("C", 1, 0, 0)],
        );
        let marked = |p: &UsagePanel| {
            lines(p)
                .into_iter()
                .find(|l| l.trim_start().starts_with('>'))
                .expect("总得有一行是选中的")
        };
        assert!(marked(&p).contains('A'), "开头选第一个");
        assert!(p.on_key(&Action::Down));
        assert!(marked(&p).contains('B'));
        assert!(p.on_key(&Action::Down));
        assert!(marked(&p).contains('C'));
        assert!(p.on_key(&Action::Down));
        assert!(marked(&p).contains('A'), "到底绕回第一个");
        assert!(p.on_key(&Action::Up));
        assert!(marked(&p).contains('C'), "往上绕回最后一个");
    }

    #[test]
    fn tab_switches_the_digit_standard() {
        let mut p = panel(UsageLang::En, vec![group("A", 24_400_000, 0, 0)]);
        assert!(lines(&p)[0].contains("24.4M"));
        assert!(p.on_key(&Action::Complete));
        assert!(lines(&p)[0].contains("2440.0万"), "Tab 换成中文口径");
        assert!(p.on_key(&Action::Complete));
        assert!(lines(&p)[0].contains("24.4M"), "再按一次换回来");
    }

    #[test]
    fn escape_closes_and_nothing_else_does() {
        let mut p = panel(UsageLang::En, vec![group("A", 1, 1, 0)]);
        assert!(!p.on_key(&Action::Left), "面板没借左键");
        assert!(p.is_open());
        assert!(p.on_key(&Action::DismissPanel));
        assert!(!p.is_open());
    }

    #[test]
    fn a_model_without_a_price_sheet_says_so_instead_of_showing_zero() {
        let mut g = group("X", 100, 10, 0);
        g.cost = None;
        g.currency = None;
        let out = lines(&panel(UsageLang::En, vec![g]));
        assert!(out[3].contains("no price sheet"), "{}", out[3]);
        assert!(!out[3].contains("0.00"), "认不出价格就不许写金额：{}", out[3]);
    }

    #[test]
    fn an_empty_session_still_prints_the_head() {
        let out = lines(&panel(UsageLang::En, Vec::new()));
        assert!(out[0].contains("Total Token"), "{}", out[0]);
        assert!(out[0].contains("0"), "没跑过就是零：{}", out[0]);
        assert!(out[1].contains("-"), "没有输入就写不出命中率：{}", out[1]);
        assert!(out[3].contains("no rounds yet"), "{}", out[3]);
    }

    fn lines_at(p: &UsagePanel, width: u16) -> Vec<String> {
        let theme = Palette::current();
        let rows = usize::from(p.want().unwrap());
        p.draw(width, rows, &theme).iter().map(text).collect()
    }

    /// `Model` 表头要和模型名的**首字符**对齐：名字前面永远站着那个 `>`，
    /// 表头那一格留空才对得上。错一列，整张表就歪了。
    #[test]
    fn the_model_header_lines_up_with_the_name_under_it() {
        let out = lines_at(&panel(UsageLang::En, vec![group("GLM5.3F(WB)", 1000, 10, 0)]), 100);
        let head_col = out[2].find("Model").expect("表头");
        let name_col = out[3].find("GLM5.3F(WB)").expect("模型行");
        assert_eq!(head_col, name_col, "表头与名字错开了：\n{}\n{}", out[2], out[3]);
        assert!(
            out[3].trim_start().starts_with('>'),
            "> 该在名字左边：{}",
            out[3]
        );
    }

    /// 金额列右对齐：`¥` 落在表头首字符的左侧（数尾对齐、符号在前）。
    #[test]
    fn the_money_columns_are_right_aligned_under_their_headers() {
        let out = lines_at(&panel(UsageLang::En, vec![group("A", 1000, 10, 0)]), 100);
        let (head, row) = (&out[2], &out[3]);
        // 比的是**显示宽度**（`¥` 占一列但两字节），不是字节下标。
        let end_of = |line: &str, from: usize, text: &str| display_width(&line[..from]) + display_width(text);
        let head_end = end_of(head, head.find("Total").unwrap(), "Total");
        let money_end = end_of(row, row.find('¥').unwrap(), "¥4.62");
        assert_eq!(head_end, money_end, "Total 列没右对齐：\n{head}\n{row}");
    }

    /// 整块的贴边方式由配置定（`config.yaml` → `usage.align`）。
    #[test]
    fn the_alignment_comes_from_the_config() {
        let mk = |align| {
            let mut p = panel(UsageLang::En, vec![group("A", 1000, 10, 0)]);
            p.set_align(align);
            lines_at(&p, 200)
        };
        // 块的左边缘 = `Model` 的位置（标记是额外的一格，不算在块里）。
        let indent = |out: &[String]| out[2].find("Model").expect("表头");
        let left = mk(UsageAlign::Left);
        let center = mk(UsageAlign::Center);
        let right = mk(UsageAlign::Right);
        assert!(indent(&left) <= MARKER_W, "左对齐贴左边（最多差一个标记位）：{}", indent(&left));
        assert!(
            indent(&left) < indent(&center) && indent(&center) < indent(&right),
            "居中也得在中间：{} / {} / {}",
            indent(&left),
            indent(&center),
            indent(&right)
        );
        // 右对齐 = 最宽的那一行顶到终端右边缘。
        let widest = right
            .iter()
            .map(|l| display_width(l.trim_end()))
            .max()
            .unwrap();
        assert_eq!(widest, 200, "右对齐时最宽的行该顶到右边：{widest}");
    }

    /// 模型行与表头的左边缘是同一条线（名字那格前面站着标记位，两边都有）。
    #[test]
    fn every_row_shares_one_left_edge() {
        let out = lines_at(&panel(UsageLang::En, vec![group("A", 1000, 10, 0)]), 160);
        let head = out[2].find("Model").expect("表头");
        let row = out[3].find('A').expect("模型行");
        assert_eq!(head, row, "表头和模型行不在同一条左边缘上：\n{}\n{}", out[2], out[3]);
    }

    /// 整块居中：左缩进处处相同（块内左对齐），而**最宽那一行**的左右留白对称
    /// ——窄的行右边多留一点，那是表该有的样子，不是没对齐。
    #[test]
    fn the_whole_block_is_centred() {
        let p = panel(UsageLang::En, vec![group("GLM5.3F(WB)", 1000, 10, 0)]);
        for width in [80u16, 120, 200] {
            let out = lines_at(&p, width);
            // 块的左边缘：`Model` 的位置减掉标记位。表头是块里最宽的一行
            // （名字列 18 + 五列 11 > 头部那两行），所以右留白按它算。
            let left = out[2].find("Model").expect("表头");
            let widest = out.iter().map(|l| display_width(l.trim_end())).max().unwrap();
            let right = usize::from(width) - widest;
            assert!(
                left.abs_diff(right) <= 1,
                "@{width} 没居中：左 {left} 右 {right}"
            );
            assert!(left > 0, "@{width} 宽屏下居然贴着左边");
            // 每一行的名字/表头都落在同一条左边缘上。
            assert_eq!(
                out[3].find('G').expect("模型行"),
                left,
                "@{width} 模型行跟表头错开了"
            );
            // 头部那两行也必须从同一条线起：`Total Tokens` 的 T 与 `Model`
            // 的 M 同列，否则三行头部自己就歪了。
            assert_eq!(
                out[0].find("Total Tokens").expect("总量行"),
                left,
                "@{width} 总量行没跟表头对齐"
            );
            assert_eq!(
                out[1].find("Cache Hit Rate").expect("命中率行"),
                left,
                "@{width} 命中率行没跟表头对齐"
            );
        }
    }

    /// 窄终端逐档收窄，但**五列一列都不许少**：用户要的是"一个模型一行看全"。
    #[test]
    fn a_narrow_terminal_steps_down_through_the_layouts_but_keeps_all_five_columns() {
        let p = panel(UsageLang::En, vec![group("GLM5.3F(WB)", 1000, 10, 0)]);
        let header = |w: u16| lines_at(&p, w)[2].trim().to_string();
        assert_ne!(
            header(90),
            header(50),
            "宽窄两档的表头宽度该不一样：{} / {}",
            header(90),
            header(50)
        );
        // 每一档都是**六列**（名字 + 五栏），名字怎么缩不影响列数——列少了
        // 就等于把"一个模型一行看全"这件事毁了。
        for w in [90u16, 70, 60, 50, 45, 40] {
            let h = header(w);
            assert_eq!(
                h.split_whitespace().count(),
                6,
                "@{w} 的列数不对（该是 Model + 五栏）：{h}"
            );
            assert!(h.contains("Model"), "@{w} 少了名字列：{h}");
        }
        // 宽敞档给全称；窄了才缩写，但缩写也是同一个词的头。
        let wide = header(90);
        for col in ["Total", "Input", "Output", "CacheRead", "CacheWrite"] {
            assert!(wide.contains(col), "宽敞档该给全称：{wide}");
        }
        assert!(
            header(50).contains("CRe") && header(50).contains("CWr"),
            "窄档要缩写：{}",
            header(50)
        );
    }

    /// 金额随列宽收窄而不是被砍残：同一个数，宽敞档写全，窄档退到两位。
    #[test]
    fn a_narrow_layout_shrinks_the_money_instead_of_chopping_it() {
        let mut g = group("A", 10, 1, 0);
        g.cost = Some(UsageCost {
            input: 0.000018,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        });
        let p = panel(UsageLang::En, vec![g]);
        let wide = lines_at(&p, 100)[3].clone();
        assert!(wide.contains("¥0.000018"), "宽敞档要写全：{wide}");
        let narrow = lines_at(&p, 50)[3].clone();
        assert!(!narrow.contains('…'), "不准写残数：{narrow}");
        assert!(
            narrow.contains("¥0.00") || narrow.contains("¥0"),
            "收窄之后数字还得在：{narrow}"
        );
    }

    /// 标记是**额外的一列**：宽度不够时整个不画，而不是挤占表格的列宽。
    /// 选中与否还有 accent 色兜底，名字的位置绝不能被它推走。
    #[test]
    fn the_marker_is_dropped_before_it_can_push_the_table_around() {
        let p = panel(UsageLang::En, vec![group("A", 1000, 10, 0)]);
        // 最宽档的表格占 73 列：74 才腾得出标记那一格，73 就腾不出。
        let tight = lines_at(&p, 73);
        assert!(!tight[3].starts_with('>'), "{}", tight[3]);
        assert_eq!(
            tight[3].find('A'),
            tight[2].find("Model"),
            "没有标记位时名字照样和表头对齐"
        );
        // 宽裕时它回来，且落在 `Model` 左边一格。
        let roomy = lines_at(&p, 100);
        assert_eq!(roomy[3].find('A'), roomy[2].find("Model"), "名字位置不变");
        assert_eq!(
            roomy[3].find('>').expect("选中标记"),
            roomy[2].find("Model").expect("表头") - MARKER_W,
            "> 在 Model 左边一格：{}",
            roomy[3]
        );
    }

    /// 极窄终端：装不下就**截断**，绝不能整行消失。这里钉的是一个真踩过的
    /// 坑——裁剪只丢 span 不截内容时，表头（整行一个 span）会变成空行。
    #[test]
    fn an_extremely_narrow_terminal_truncates_instead_of_swallowing_rows() {
        let p = panel(UsageLang::En, vec![group("GLM5.3F(WB)", 1000, 10, 0)]);
        let out = lines_at(&p, 20);
        assert!(out[0].contains("Tot"), "头部那行被吞了：{:?}", out[0]);
        assert!(!out[2].trim().is_empty(), "表头被吞了");
        // 20 列时标记那一格腾不出来，于是它不画——但名字必须在。
        assert!(out[3].contains("GLM"), "名字列该活下来：{:?}", out[3]);
    }

    #[test]
    fn a_narrow_terminal_never_overflows_the_row() {
        let p = panel(
            UsageLang::En,
            vec![group("a-very-long-model-name-here", 1_200_000, 40_000, 0)],
        );
        let theme = Palette::current();
        for w in [30u16, 60, 100] {
            for line in p.draw(w, usize::from(p.want().unwrap()), &theme) {
                assert!(
                    line.spans.iter().map(|s| s.content.chars().count()).sum::<usize>()
                        <= usize::from(w),
                    "@{w} 越界：{}",
                    text(&line)
                );
            }
        }
    }
}
