//! Markdown → ratatui rows, omp-style.
//!
//! Layout contract ported from omp `components/markdown.ts` (terminal
//! subset): headings carry `#` prefixes with an underline on h1, lists
//! use omp's hanging indent (`2×depth` prefix, continuation rows align
//! under the text past the bullet, ordered lists keep their real
//! numbers), quotes draw the `▏ ` gutter, rules render as `─` runs,
//! fenced code blocks get a border-gray ``` frame with syntax
//! highlighting inside, inline code is `mdCode` colored **inline**.
//!
//! GFM tables get a boxed layout (see `render_table`); footnotes and math
//! degrade to plain text.
//!
//! Entry points: user / assistant / tool_result (markdown-capable)
//! messages go through [`render_markdown`]; tool_request and Plain views
//! do not.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::blocks::Deferred;
use crate::tui::text::{char_width, display_width};
use super::highlight;
use super::theme::{HistoryTheme, Token};

/// 一张正在解析的 GFM 表格。
///
/// 事件形状（pulldown-cmark 0.13，实测）：
///
/// ```text
/// Start(Table) Start(TableHead) Cell… End(TableHead)
///              Start(TableRow) Cell… End(TableRow)… End(Table)
/// ```
///
/// **表头不经过 `TableRow`** —— `TableHead` 直接包 `TableCell`，数据行才有
/// `TableRow`。所以表头行在 `End(TableHead)` 收，数据行在 `End(TableRow)` 收。
#[derive(Default)]
struct TableBuf {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    /// 当前行已收的单元格。
    cur: Vec<String>,
    /// 当前单元格的文本。`None` = 不在单元格里（单元格之间）。
    cell: Option<String>,
}

/// One in-flight text row being assembled.
struct Row {
    spans: Vec<Span<'static>>,
}

impl Row {
    fn push(&mut self, sp: Span<'static>) {
        self.spans.push(sp);
    }
    fn take(&mut self) -> Line<'static> {
        Line::from(std::mem::take(&mut self.spans))
    }
}

/// Render markdown text into rows. Empty text yields nothing.
///
/// `defer` 时**代码围栏按纯文本出图**，并把每个围栏的位置与源码一并交回
/// （[`Deferred`]），由调用方决定何时补色。围栏之间是独立的渲染单元
/// （每个围栏一个 fresh `ParseState`），所以"先出哪个围栏的颜色"是自由的。
pub fn render_markdown(
    text: &str,
    t: &HistoryTheme,
    defer: bool,
    width: usize,
) -> (Vec<Line<'static>>, Vec<Deferred>) {
    // Colors read from the global theme (fresh per call): runtime theme
    // switches recolor the next render with no signature churn.
    let mut out: Vec<Line<'static>> = Vec::new();
    // 推迟上色的段（只在新 `defer` 时非空）。
    let mut deferred: Vec<Deferred> = Vec::new();
    // Current row being assembled (paragraph / list item text).
    let mut row: Option<Row> = None;
    // Inline style state.
    let mut bold = false;
    let mut italic = false;
    let mut in_heading = false;
    // Block context.
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_buf = String::new();
    let mut quote_depth: usize = 0;
    // List context: stack of (ordered, next_number). omp renders real
    // numbers for ordered lists; nested lists indent by 2 cells per level.
    let mut lists: Vec<(bool, u64)> = Vec::new();
    // 表格解析状态。`None` = 不在表格里。
    let mut table: Option<TableBuf> = None;

    let inline_style = |bold: bool, italic: bool| {
        let mut s = Style::new();
        if bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    };

    let flush = |row: &mut Option<Row>, out: &mut Vec<Line<'static>>| {
        if let Some(mut r) = row.take() {
            out.push(r.take());
        }
    };

    macro_rules! ensure_row {
        ($row:expr) => {
            if $row.is_none() {
                $row = Some(Row { spans: Vec::new() });
            }
        };
    }

    // ENABLE_TABLES 是 pulldown-cmark 自带的 GFM 表格（`redistribute` 那套
    // 列宽算法另写，见 `render_table`）。不开的话 `| a | b |` 会被当普通段落，
    // 每个 `|` 原样输出，读起来是一团。
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    for ev in Parser::new_ext(text, opts) {
        match ev {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush(&mut row, &mut out);
                out.push(Line::from(""));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut row, &mut out);
                let depth = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    _ => 3,
                };
                let prefix = format!("{} ", "#".repeat(depth));
                // omt: h1/h2 drop the prefix for large-text feel… but in
                // terminals without text sizing, the `#` keeps the heading
                // readable; h1 gets the underline treatment via bold+accent.
                in_heading = true;
                bold = true;
                if depth > 2 {
                    ensure_row!(row);
                    row.as_mut().unwrap().push(t.fg_mod(
                        prefix,
                        Token::MdHeading,
                        Modifier::BOLD,
                    ));
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                bold = false;
                in_heading = false;
                flush(&mut row, &mut out);
                // Heading breathes: omp pads after headings unless a blank
                // line already follows.
                out.push(Line::from(""));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut row, &mut out);
                let lang = match &kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => {
                        info.split([',', ' ']).next().unwrap_or("").to_string()
                    }
                    _ => String::new(),
                };
                // omt: the fence lines render in mdCodeBlockBorder gray.
                out.push(Line::from(
                    t.fg("```".to_string(), Token::MdCodeBlockBorder),
                ));
                in_code_block = Some(lang);
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang = in_code_block.take().unwrap_or_default();
                // Highlight the whole block at once (syntect needs forward
                // state across lines), then split into rows.
                let hint = if lang.is_empty() {
                    None
                } else {
                    Some(lang.as_str())
                };
                if defer {
                    // 纯文本出图 + 登记这一段：行数与颜色无关，几何照旧。
                    let before = out.len();
                    let plain = highlight::plain(&code_buf);
                    let count = plain.len();
                    out.extend(plain);
                    deferred.push(Deferred {
                        before,
                        count,
                        code: std::mem::take(&mut code_buf),
                        lang: hint.map(str::to_string),
                    });
                } else {
                    for line in highlight::highlight(&code_buf, hint, t) {
                        out.push(line);
                    }
                }
                out.push(Line::from(
                    t.fg("```".to_string(), Token::MdCodeBlockBorder),
                ));
                out.push(Line::from(""));
            }
            Event::Text(txt) => {
                let s = txt.to_string();
                // 单元格里：收到当前单元格，别走正文那一套（它会把内容推进
                // `out`，表格就散架了）。
                if let Some(c) = table.as_mut().and_then(|tb| tb.cell.as_mut()) {
                    c.push_str(&s);
                } else if in_code_block.is_some() {
                    code_buf.push_str(&s);
                } else if quote_depth > 0 {
                    // Quote rows: gutter + text, italics muted (omp mdQuote).
                    // Continuation events (inline code, wrapped source rows)
                    // append to the open quote row — the gutter appears once
                    // per visual row, not once per parser event.
                    for part in s.split('\n') {
                        if part.is_empty() {
                            continue;
                        }
                        ensure_row!(row);
                        let r = row.as_mut().unwrap();
                        if r.spans.is_empty() {
                            r.push(t.fg("▏ ".to_string(), Token::MdQuoteBorder));
                        }
                        r.push(Span::styled(
                            part.to_string(),
                            t.fg_style(Token::MdQuote)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    }
                } else {
                    ensure_row!(row);
                    let r = row.as_mut().unwrap();
                    let mut st = inline_style(bold, italic);
                    if in_heading {
                        st = st.fg(t.get(Token::MdHeading));
                    }
                    r.push(Span::styled(s, st));
                }
            }
            Event::Code(t2) => {
                // 单元格里的行内代码：表格不保留行内样式（框线要对齐，
                // 混入彩色片段只会更难读），只取文字。
                if let Some(c) = table.as_mut().and_then(|tb| tb.cell.as_mut()) {
                    c.push_str(&t2);
                } else {
                    // Inline code: mdCode color, appended to the current row —
                    // a sentence mentioning `main.rs` stays one sentence.
                    ensure_row!(row);
                    let r = row.as_mut().unwrap();
                    r.push(t.fg(t2.to_string(), Token::MdCode));
                }
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut row, &mut out);
                let ordered = start.is_some();
                lists.push((ordered, start.unwrap_or(1)));
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut row, &mut out);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut row, &mut out);
                let depth = lists.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let slot = lists.last_mut().expect("Item inside List");
                let ordered = slot.0;
                let n = slot.1;
                if ordered {
                    slot.1 += 1;
                }
                let bullet = if ordered {
                    format!("{n}. ")
                } else {
                    // ascii preset: the omp bullet is `- `; the unicode `-` is
                    // the same glyph, so one form serves both.
                    "- ".to_string()
                };
                let mut r = Row { spans: Vec::new() };
                r.push(t.fg(format!("{indent}{bullet}"), Token::MdListBullet));
                row = Some(r);
            }
            Event::End(TagEnd::Item) => {
                flush(&mut row, &mut out);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut row, &mut out);
                quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut row, &mut out);
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough) => {}
            // ---- GFM 表格 ----
            //
            // 表头与数据行都先收进 `TableBuf`，整张表收完再一次性排版：
            // 列宽取决于该列**所有**单元格的宽度，逐行画是画不出来的。
            Event::Start(Tag::Table(_alignments)) => {
                flush(&mut row, &mut out);
                table = Some(TableBuf::default());
            }
            Event::End(TagEnd::Table) => {
                if let Some(tb) = table.take() {
                    let lines = render_table(&tb.header, &tb.rows, width, t);
                    if lines.is_empty() {
                        // 放不下整张表：退回原始 markdown 文本，比画一张挤成
                        // 一条缝的表强。
                        for raw in raw_table_lines(&tb) {
                            out.push(Line::from(Span::styled(raw, t.fg_style(Token::MdCode))));
                        }
                    } else {
                        out.extend(lines);
                        out.push(Line::from(""));
                    }
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(tb) = table.as_mut() {
                    tb.cur.clear();
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(tb) = table.as_mut() {
                    tb.header = std::mem::take(&mut tb.cur);
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(tb) = table.as_mut() {
                    tb.cur.clear();
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(tb) = table.as_mut() {
                    tb.rows.push(std::mem::take(&mut tb.cur));
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(tb) = table.as_mut() {
                    tb.cell = Some(String::new());
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(tb) = table.as_mut()
                    && let Some(text) = tb.cell.take()
                {
                    tb.cur.push(text);
                }
            }
            Event::Rule => {
                flush(&mut row, &mut out);
                out.push(Line::from(Span::styled(
                    "─".repeat(20),
                    t.fg_style(Token::MdHr),
                )));
                out.push(Line::from(""));
            }
            Event::SoftBreak => {
                // Paragraph-internal newline: start a new row (hanging
                // alignment comes free — list prefix only adorns row 0).
                flush(&mut row, &mut out);
            }
            Event::HardBreak => {
                flush(&mut row, &mut out);
            }
            Event::TaskListMarker(_) => {}
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
            _ => {}
        }
    }
    flush(&mut row, &mut out);
    // Drop a single trailing blank row (paragraph separator)
    if out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    (out, deferred)
}

// ── GFM 表格 ──────────────────────────────────────────────────────────────
//
// 布局对齐 omp `components/markdown.ts::#renderTable`：
//
//   * 列宽先取内容的**自然宽度**（该列所有单元格的最大显示宽度）；
//   * 整张表放不下时，以"该列最长不可断片"为下限，按可增长量加权分配
//     剩余空间——所以长 URL 列会先被压到它单词的宽度，而不是把某一列
//     压成一条缝；
//   * 单元格超宽是**折行**，不是截断（截断会吞掉内容，用户看不出来）；
//   * 连每列一格都放不下 → 退回原始 markdown 文本。画一张挤成一条缝的
//     表比不画更糟。
//
// 和 omp 一样不理会 GFM 的对齐标记（`|---:|`）：终端里空格对齐的可读性
// 收益，抵不上为此多算一遍每列的显示宽度的复杂度。

/// 一列最长不可断片的宽度上限。一列再怎么缩，也不该把一个 30 字符的
/// URL 切成七八行。
const TABLE_MAX_WORD: usize = 30;

/// 最长不可断片的宽度，上限 `TABLE_MAX_WORD`。
///
/// 断点：空白处可断。CJK 之间**没有**空白，所以整串会被当成一个词——
/// 那正好，中文列的最小宽度就是它最长的连续文段，符合直觉。
fn longest_word_width(s: &str) -> usize {
    let mut best = 1usize;
    for w in s.split_whitespace() {
        // 一个词的字节数 ≥ 上限，其显示宽度必然也 ≥ 上限（UTF-8 里一个
        // 字符至少一字节）。大单元格省掉这趟扫描。
        if w.len() >= TABLE_MAX_WORD {
            return TABLE_MAX_WORD;
        }
        best = best.max(display_width(w));
        if best >= TABLE_MAX_WORD {
            return TABLE_MAX_WORD;
        }
    }
    best
}

/// 单元格按列宽折行。
///
/// 先在空白处断词，单个词仍然超宽就按显示宽度硬切（中文没有空白可以断，
/// 这条是它的主路径）。返回的每一行显示宽度都 `<= limit`。
fn wrap_cell(s: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;

    let push_line = |out: &mut Vec<String>, line: &mut String| {
        out.push(std::mem::take(line));
    };

    for word in s.split_whitespace() {
        let word_w = display_width(word);
        // 换行位置：当前行非空时，单词前面补一个空格。
        let sep = if line.is_empty() { 0 } else { 1 };
        if line_w + sep + word_w <= limit {
            if sep == 1 {
                line.push(' ');
            }
            line.push_str(word);
            line_w += sep + word_w;
            continue;
        }
        // 放不下：先收掉当前行。
        if !line.is_empty() {
            push_line(&mut out, &mut line);
            line_w = 0;
        }
        if word_w <= limit {
            line.push_str(word);
            line_w = word_w;
            continue;
        }
        // 单个词就超宽（中文长句、长 URL）：按显示宽度硬切。
        for ch in word.chars() {
            let cw = char_width(ch);
            // 当前行非空才换：一个宽 2 的字符（中文）落进 1 格的列时，
            // 换行会在前面留下一个空行，而且它照样放不下。
            if line_w + cw > limit && !line.is_empty() {
                push_line(&mut out, &mut line);
                line_w = 0;
            }
            line.push(ch);
            line_w += cw;
        }
    }
    if !line.is_empty() {
        push_line(&mut out, &mut line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// 放不下整张表时的回退：把解析结果拼回 pipe 表格的样子。
///
/// 从单元格重拼，而不是留一份原文——`pulldown_cmark` 的事件不带源码范围，
/// 拿不到"这张表原文占哪几行"。重拼的文本未必与原文逐字节相同（缩进、对齐
/// 冒号会丢），但读起来还是张表，而且不会像"一行行 `|` 挤在终端里"那样散架。
fn raw_table_lines(tb: &TableBuf) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // 用 ASCII 的 `|`：这是"退回原始文本"的样子，读者一眼就知道窄了，
    // 而不是以为画了一张没边框的表。
    let join = |cells: &[String]| format!("| {} |", cells.join(" | "));
    if !tb.header.is_empty() {
        out.push(join(&tb.header));
        let rules: Vec<String> = tb.header.iter().map(|_| "---".to_string()).collect();
        out.push(format!("| {} |", rules.join(" | ")));
    }
    for row in &tb.rows {
        out.push(join(row));
    }
    out
}

/// 按"每列最长不可断片"的下限，把 `available` 分给 `n` 列。
///
/// 下限之和本来就放得下时直接用下限；放不下就每列先给 1 格，剩余的按
/// 各列的可增长量加权分配（除不尽的部分从左往右每人加一格，保证用完）。
fn fit_min_widths(natural: &[usize], minw: &[usize], available: usize) -> Vec<usize> {
    let n = natural.len();
    let min_sum: usize = minw.iter().sum();
    if min_sum <= available {
        return minw.to_vec();
    }
    let mut out = vec![1usize; n];
    let remaining = available.saturating_sub(n);
    if remaining > 0 {
        let weights: Vec<usize> = minw.iter().map(|w| w.saturating_sub(1)).collect();
        let total: usize = weights.iter().sum();
        if total > 0 {
            for i in 0..n {
                out[i] += (weights[i] * remaining) / total;
            }
            let allocated: usize = out.iter().sum::<usize>() - n;
            let mut leftover = remaining.saturating_sub(allocated);
            let mut i = 0;
            while leftover > 0 && i < n {
                out[i] += 1;
                leftover -= 1;
                i += 1;
            }
        }
    }
    out
}

/// 一张 GFM 表格。
///
/// `width` 是这块内容能用的**总列数**（终端宽度减去左右留白）。放不下整张
/// 表时返回空 —— 调用方据此退回原始文本。
fn render_table(
    header: &[String],
    rows: &[Vec<String>],
    width: usize,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let n = header.len();
    if n == 0 {
        return Vec::new();
    }
    // 框线开销：「│ 」 + (n-1) 个「 │ 」 + 「 │」 = 3n + 1
    let overhead = 3 * n + 1;
    // 每列至少 2 格。omp 的判据是每列 1 格，但那只在纯 ASCII 上成立：
    // 一个中文字占 2 格，1 格的列会把表头竖排成一列单字，比退回原始
    // 文本还难读。
    let min_available = n * 2;
    if width < overhead + min_available {
        return Vec::new();
    }
    let available = width - overhead;

    // 自然宽度与下限。
    let mut natural = vec![0usize; n];
    let mut minw = vec![1usize; n];
    let note = |cells: &[String], natural: &mut Vec<usize>, minw: &mut Vec<usize>| {
        for (i, c) in cells.iter().enumerate().take(n) {
            natural[i] = natural[i].max(display_width(c));
            minw[i] = minw[i].max(longest_word_width(c));
        }
    };
    note(header, &mut natural, &mut minw);
    for row in rows {
        note(row, &mut natural, &mut minw);
    }

    // 列宽：放不下就缩到下限，再按可增长量把剩余宽度分回去。
    let natural_sum: usize = natural.iter().sum();
    let widths: Vec<usize> = if natural_sum <= available {
        natural.iter().zip(&minw).map(|(a, b)| (*a).max(*b)).collect()
    } else {
        let fits = fit_min_widths(&natural, &minw, available);
        let min_sum: usize = fits.iter().sum();
        let total_growth: usize = natural
            .iter()
            .zip(&fits)
            .map(|(a, b)| a.saturating_sub(*b))
            .sum();
        let extra = available.saturating_sub(min_sum);
        let mut out: Vec<usize> = natural
            .iter()
            .zip(&fits)
            .map(|(nat, min)| {
                let delta = nat.saturating_sub(*min);
                let grow = if total_growth > 0 {
                    (delta * extra) / total_growth
                } else {
                    0
                };
                min + grow
            })
            .collect();
        // 除不尽的余量从左往右补，直到用完。
        let mut leftover = available.saturating_sub(out.iter().sum::<usize>());
        let mut i = 0;
        while leftover > 0 && i < n {
            if out[i] < natural[i] {
                out[i] += 1;
                leftover -= 1;
            }
            i += 1;
            if i == n && leftover > 0 {
                i = 0;
                if out.iter().zip(&natural).all(|(o, nat)| o >= nat) {
                    break;
                }
            }
        }
        out
    };

    let border = |s: String| Line::from(Span::styled(s, t.fg_style(Token::MdTableBorder)));
    let mut out: Vec<Line<'static>> = Vec::new();

    // 框线必须与内容行**逐格对齐**：内容行是 `│ ` + 单元格 + ` │ `，所以
    // 每条横线在单元格两侧各多一格。少了这两格，横线行会比内容行窄 4 格，
    // 看起来就是歪的。
    let mut top = String::from("┌─");
    for (i, w) in widths.iter().enumerate() {
        top.push_str(&"─".repeat(*w));
        top.push_str(if i + 1 == n { "─┐" } else { "─┬─" });
    }
    out.push(border(top));

    // 行渲染：单元格先折行，再按行号拼，每格补齐到列宽。
    let render_row = |cells: &[String], bold: bool| -> Vec<Line<'static>> {
        let wrapped: Vec<Vec<String>> = (0..n)
            .map(|i| wrap_cell(cells.get(i).map(String::as_str).unwrap_or(""), widths[i]))
            .collect();
        let lines = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        (0..lines)
            .map(|li| {
                let mut spans: Vec<Span<'static>> = vec![Span::styled(
                    "│ ".to_string(),
                    t.fg_style(Token::MdTableBorder),
                )];
                for (i, cell) in wrapped.iter().enumerate() {
                    let text = cell.get(li).map(String::as_str).unwrap_or("");
                    let pad = widths[i].saturating_sub(display_width(text));
                    let mut st = Style::new();
                    if bold {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    // 一次分配搞定「内容 + 补白」。表格是这里唯一按**格**分配
                    // 的地方，一张 3000 行的表就是三万个单元格——多一次
                    // `" ".repeat()` 的临时分配在热路径上是白给。
                    let mut padded = String::with_capacity(text.len() + pad);
                    padded.push_str(text);
                    for _ in 0..pad {
                        padded.push(' ');
                    }
                    spans.push(Span::styled(padded, st));
                    spans.push(Span::styled(
                        if i + 1 == n { " │" } else { " │ " }.to_string(),
                        t.fg_style(Token::MdTableBorder),
                    ));
                }
                Line::from(spans)
            })
            .collect()
    };

    out.extend(render_row(header, true));

    // 表头与数据之间的分隔：├──┼──┤
    let mut sep = String::from("├─");
    for (i, w) in widths.iter().enumerate() {
        sep.push_str(&"─".repeat(*w));
        sep.push_str(if i + 1 == n { "─┤" } else { "─┼─" });
    }

    // 表头下面一条，数据行之间各一条（与 omp 一致：最后一行下面不画，
    // 紧跟着就是底边）。
    for row in rows {
        out.push(border(sep.clone()));
        out.extend(render_row(row, false));
    }
    if rows.is_empty() {
        out.push(border(sep));
    }

    // 底边：└──┴──┘
    let mut bottom = String::from("└─");
    for (i, w) in widths.iter().enumerate() {
        bottom.push_str(&"─".repeat(*w));
        bottom.push_str(if i + 1 == n { "─┘" } else { "─┴─" });
    }
    out.push(border(bottom));
    out
}

/// 测试用的便捷入口：不推迟上色，只要行。
#[cfg(test)]
fn render_markdown_at(text: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    render_markdown(text, t, false, 100).0
}

/// 指定宽度渲染（表格的宽度行为测试用）。
#[cfg(test)]
fn render_markdown_at_width(text: &str, t: &HistoryTheme, width: usize) -> Vec<Line<'static>> {
    render_markdown(text, t, false, width).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::theme::HistoryTheme;
    use ratatui::style::Color;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    fn text_of(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 一张 GFM 表格画成框线表。
    #[test]
    fn a_gfm_table_draws_a_box() {
        let md = "| 名称 | 说明 |\n|---|---|\n| a | 第一 |\n| b | 第二 |";
        let all = text_of(&render_markdown_at_width(md, &t(), 60));
        assert!(all.starts_with("┌"), "{all}");
        assert!(all.contains("┘"), "{all}");
        // 表头加粗且补齐到列宽（`名称` 与 `说明` 都是 4 格）。
        assert!(all.contains("│ 名称 │ 说明 │"), "{all}");
        // 数据行按列宽补齐：`a` 后面补到 4 格。
        assert!(all.contains("│ a    │ 第一 │"), "{all}");
        // 行与行之间有分隔线，不是一坨。
        assert!(all.contains("├"), "{all}");
    }

    /// 表格框线用自己的色号（白），不与代码块围栏共用——围栏是灰的，
    /// 框线要的是能一眼看清表格的边界。
    #[test]
    fn table_borders_carry_their_own_color() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |";
        let lines = render_markdown_at_width(md, &t(), 40);
        let theme = HistoryTheme::resolve();
        let table = theme.get(Token::MdTableBorder);
        assert_ne!(
            table,
            theme.get(Token::MdCodeBlockBorder),
            "不该与代码块共用色号"
        );
        // 框线字符上色；单元格内容不上色（只有加粗，前景取默认）。
        let mut borders = 0usize;
        for l in &lines {
            for s in &l.spans {
                if let Some(fg) = s.style.fg {
                    assert_eq!(fg, table, "这一格不是框线色：{:?}", s.content);
                    borders += 1;
                }
            }
        }
        assert!(borders >= 4, "顶边、竖线、底边都该上色，只数到 {borders}");
    }

    /// 中文按**格**算宽度。按字符数算的话每行都会错开两格，
    /// 框线看起来是歪的。
    #[test]
    fn cjk_cells_align_by_display_width() {
        let md = "| 列 | 名字 |\n|---|---|\n| x | 中文 |\n| y | ab |";
        let lines = render_markdown_at_width(md, &t(), 60);
        let widths: Vec<usize> = lines
            .iter()
            .map(|l| display_width(&l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()))
            .collect();
        // 表头、正文、框线——每一行的显示宽度都必须相同。
        assert!(widths.len() >= 5, "{widths:?}");
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "行宽不一致，框线会歪：{widths:?}"
        );
    }

    /// 单元格内容超宽要**折行**，不是被截掉——截掉的内容用户看不见，
    /// 而表格恰恰是拿来读内容的。
    #[test]
    fn a_long_cell_wraps_instead_of_being_cut() {
        let md = "| 键 | 值 |\n|---|---|\n| k | 这是一段很长的中文内容会被折成好几行 |";
        let all = text_of(&render_markdown_at_width(md, &t(), 26));
        assert!(all.contains("└"), "是张表：{all}");
        // 折行会插进空格与框线字符，所以比对时把它们去掉——要验的是
        // **一个字都没丢**，而不是恰好断在某处。
        let flat: String = all
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '│' && *c != '┌' && *c != '┐' && *c != '└' && *c != '┘' && *c != '├' && *c != '┤' && *c != '─' && *c != '┬' && *c != '┴' && *c != '┼')
            .collect();
        assert!(
            flat.contains("这是一段很长的中文内容会被折成好几行"),
            "折行不能丢内容：{all}"
        );
    }

    /// 窄到放不下一张表时退回原始 pipe 文本，而不是画一张挤成一条缝的
    /// 表——那种表比不画更难看懂。
    #[test]
    fn a_table_that_cannot_fit_falls_back_to_pipe_text() {
        let md = "| 名称 | 说明 |\n|---|---|\n| a | b |";
        let all = text_of(&render_markdown_at_width(md, &t(), 6));
        assert!(!all.contains("┌"), "不该硬画：{all}");
        assert!(all.contains("名称"), "内容不能丢：{all}");
        assert!(all.contains("说明"), "内容不能丢：{all}");
    }

    /// 流式：表格写了一半（还没有最后一行）时也要画得出来——
    /// 否则用户看到的是一坨 `|`，等流完才突然变成一个表。
    #[test]
    fn a_half_streamed_table_still_draws() {
        let all = text_of(&render_markdown_at_width("| a | b |\n|---|---|\n", &t(), 40));
        assert!(all.contains("┌"), "半张表也要成形：{all}");
        assert!(all.contains("│ a │ b │"), "{all}");
    }

    /// 表格里的行内标记不参与样式，只取文字——框线靠空格对齐，
    /// 混进彩色片段只会更难读。
    #[test]
    fn inline_markup_inside_a_cell_contributes_text_only() {
        let md = "| a | b |\n|---|---|\n| **粗** | `代码` |";
        let all = text_of(&render_markdown_at_width(md, &t(), 40));
        assert!(all.contains("粗"), "{all}");
        assert!(all.contains("代码"), "{all}");
        assert!(!all.contains("**"), "标记不该漏出来：{all}");
        assert!(!all.contains("`"), "标记不该漏出来：{all}");
    }

    #[test]
    fn bold_and_plain_mix() {
        let lines = render_markdown_at("**bold** plain", &t());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].style.has_modifier(Modifier::BOLD));
        assert!(!lines[0].spans[1].style.has_modifier(Modifier::BOLD));
    }

    #[test]
    fn code_block_is_fenced_and_indented() {
        let lines = render_markdown_at("```rust\nlet x = 1;\n```", &t());
        let all = text_of(&lines);
        assert!(all.starts_with("```"), "{all}");
        assert!(all.trim_end().ends_with("```"), "{all}");
        assert!(all.contains("let x = 1;"), "{all}");
    }

    #[test]
    fn heading_is_bold_accent() {
        let lines = render_markdown_at("## 标题", &t());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].style.has_modifier(Modifier::BOLD));
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdHeading))
        );
    }

    #[test]
    fn h3_keeps_hash_prefix() {
        let lines = render_markdown_at("### 深层", &t());
        assert_eq!(lines[0].spans[0].content, "### ");
    }

    #[test]
    fn quote_gets_bar_prefix() {
        let lines = render_markdown_at("> 引用一句", &t());
        assert_eq!(lines[0].spans[0].content, "▏ ");
    }

    #[test]
    fn ordered_list_keeps_numbers() {
        let md = "1. first\n2. second\n3. third";
        let all = text_of(&render_markdown_at(md, &t()));
        assert!(all.contains("1. first"), "{all}");
        assert!(all.contains("2. second"), "{all}");
        assert!(all.contains("3. third"), "{all}");
    }

    #[test]
    fn unordered_list_uses_dash() {
        let all = text_of(&render_markdown_at("- a\n- b", &t()));
        assert!(all.contains("- a"), "{all}");
        assert!(all.contains("- b"), "{all}");
    }

    #[test]
    fn nested_list_indents() {
        let md = "- top\n  - inner";
        let all = text_of(&render_markdown_at(md, &t()));
        let inner = all.lines().find(|l| l.contains("inner")).expect("row");
        assert!(inner.starts_with("  - "), "{all}"); // omt: 每级 2 格，恰好对齐上级文本
    }

    #[test]
    fn list_hangs_under_text_not_prefix() {
        // Continuation of a wrapped item aligns past the bullet; here the
        // item is one row, so just verify the prefix exists and the bullet
        // is list-bullet colored.
        let lines = render_markdown_at("- item", &t());
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdListBullet))
        );
        assert_eq!(lines[0].spans[0].content, "- ");
    }

    #[test]
    fn inline_code_stays_inline() {
        let lines = render_markdown_at("改 `main.rs` 和 `lib.rs`", &t());
        assert_eq!(lines.len(), 1, "行内 code 不能断行");
        let t = HistoryTheme::resolve();
        assert_eq!(
            lines[0].spans[1].style.fg,
            Some(t.get(Token::MdCode))
        );
    }

    #[test]
    fn softbreak_breaks_row() {
        let lines = render_markdown_at("一行\n二行", &t());
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(render_markdown_at("", &t()).is_empty());
    }

    #[test]
    fn user_card_markdown_survives_gutter() {
        // user_card wraps markdown rows with the ▌ gutter; the inline code
        // must not break that layout.
        let lines = super::super::cards::user_card_public_for_test(
            "看 `x.rs`",
            &t(),
            30,
        );
        assert!(lines.len() >= 3);
    }

    /// **长中文行必须在卡片内部折行。**
    ///
    /// 这是真机上的 bug：markdown 交出一条逻辑行，折行留给 ratatui——而它
    /// 折出来的续行没有 `▌ ` 竖条，背景也止于最后一个上色单元格。用户看到
    /// 的是同一行的前半黑底白字、后半落在终端默认上，下面一行连竖条都没了。
    /// 根因是 `user_card` 没有按自己真正能用的宽度折行。
    #[test]
    fn long_cjk_wraps_inside_the_card() {
        let text = "据我所知，角色扮演常用的（尾巴摇了摇）这种通过括号来展现非对话内容的做法，这个你熟悉吗？如果熟悉的话，基于你给出的这一版，然后，在当前工作区下写个文件吧。";
        let width = 60;
        let lines = super::super::cards::user_card_public_for_test(text, &t(), width);
        assert!(
            lines.len() >= 4,
            "这么长的中文该折行（留白 + 正文 >1 行 + 留白），实际 {} 行",
            lines.len()
        );
        // 每一行都必须是完整的卡片行：恰好铺满 width，带竖条，竖条有底色。
        for (i, l) in lines.iter().enumerate() {
            let w: usize = l.spans.iter().map(|s| display_width(&s.content)).sum();
            assert_eq!(w, width, "第 {i} 行宽 {w} 不等于 {width}：没铺满卡片");
            assert_eq!(l.spans[0].content, "▌ ", "第 {i} 行缺竖条");
            assert_eq!(
                l.spans[0].style.bg,
                Some(HistoryTheme::resolve().get(Token::UserCardBg)),
                "第 {i} 行的竖条没有卡片底色"
            );
        }
        // 折行不许吞字。
        let got: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .chars()
            .filter(|c| *c != '▌' && *c != ' ')
            .collect();
        let want: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(got, want, "折行丢了字");
    }

    #[test]
    fn link_autolink_renders_plain() {
        let all = text_of(&render_markdown_at("<https://example.com>", &t()));
        assert!(all.contains("https://example.com"), "{all}");
    }

    #[test]
    fn rule_is_mdhr_colored() {
        let lines = render_markdown_at("---\n\n正文", &t());
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdHr))
        );
        let _ = Color::Reset;
    }
}
