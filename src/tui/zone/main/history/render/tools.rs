//! 逐工具渲染表 —— 工具名片按工具名分派。
//!
//! 一个工具块 = header 行 + 若干**段落**（[`Section`]）。段落可选带标签，
//! 带标签就自己起一条横杠（fetch 的 `metadata` / `content` 就是这么分的）。
//! header 里四段：字形、工具名、`intent`（一句话说明这次要干什么，模型
//! 必填）、meta（零散小事实，`·` 连接）。
//!
//! 具名画师：bash、edit/mass_edit、read、fetch、browser、search；其余落
//! [`default_call`] 兜底（参数转 JSON、结果原文）。加一个工具的画师 =
//! 加一个 match 分支，不碰框、不碰缓存。
//!
//! 取值照抄 omp `packages/tui/src/tools/*.ts` 的 `renderCall`/`renderResult`
//! 分层，但只取画法：异步、spinner、图片、记忆化都还没接。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

use super::glyphs as g;
use super::highlight;
use super::theme::{HistoryTheme, Token};

/// 工具结果在 UI 眼里的形状 —— **派生自工具自己的 `details`**，不是从文本猜的。
///
/// 这是个 UI 概念，所以住在渲染层：服务端的数据模型不认识"diff"，只认识
/// 工具交上来的结构化载荷。`details` 缺席时（工具没交，或这一行是 `details`
/// 字段存在之前写进库里的老数据）退回纯文本 —— 老会话照旧能看。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ToolView {
    /// 纯文本，不走 markdown，超行折叠。
    Plain { text: String },
    /// 行级 diff：删除红、插入绿，左侧带**行号栏**（omp 的 `-315│…`）。
    Diff {
        deletions: Vec<String>,
        insertions: Vec<String>,
        /// 被改动那段在文件里的起始行（1 起）。`None` = 工具没说，
        /// 行号栏就只有标记列。
        at_line: Option<usize>,
    },
}

impl ToolView {
    /// 从 `details` 读；读不到就是纯文本。
    pub(super) fn from_details(details: Option<&serde_json::Value>, text: &str) -> ToolView {
        let diff = details.filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("diff"));
        if let Some(d) = diff {
            let lines = |key: &str| -> Vec<String> {
                d.get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(String::from)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            let (deletions, insertions) = (lines("deletions"), lines("insertions"));
            let at_line = d
                .get("atLine")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            if !deletions.is_empty() || !insertions.is_empty() {
                return ToolView::Diff {
                    deletions,
                    insertions,
                    at_line,
                };
            }
        }
        ToolView::Plain {
            text: text.to_string(),
        }
    }
}

/// 一次工具调用的结果，卡片需要的全部字段。
///
/// `details` 是工具交上来的结构化载荷（原样透传，渲染层按 `kind` 自己解释）；
/// `duration_ms` 由循环测，所以每个工具都有，与工具是否自己计时无关。
#[derive(Debug, Clone, Copy)]
pub(super) struct ToolOutcome<'a> {
    pub text: &'a str,
    pub ok: bool,
    pub details: Option<&'a serde_json::Value>,
    pub duration_ms: u64,
}

impl<'a> ToolOutcome<'a> {
    /// 测试与老调用点的简写：只有文本和成败。
    #[cfg(test)]
    pub(super) fn text(text: &'a str, ok: bool) -> Self {
        Self {
            text,
            ok,
            details: None,
            duration_ms: 0,
        }
    }

    /// 一次工具调用跑了多久 —— 只有"值得等"的调用才报（omp 的规矩：给 task/
    /// wait 这类报，不给每个 read 报）。一秒以下不占位。
    pub(super) fn duration_label(&self) -> Option<String> {
        (self.duration_ms >= 1_000).then(|| format_duration(self.duration_ms))
    }
}

/// omp `packages/utils/src/format.ts:10` 的 `formatDuration`，照抄。
pub(super) fn format_duration(ms: u64) -> String {
    const SEC: u64 = 1_000;
    const MIN: u64 = 60 * SEC;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    if ms < SEC {
        return format!("{ms}ms");
    }
    if ms < MIN {
        return format!("{:.1}s", ms as f64 / SEC as f64);
    }
    if ms < HOUR {
        let (m, s) = (ms / MIN, (ms % MIN) / SEC);
        return if s > 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        };
    }
    if ms < DAY {
        let (h, m) = (ms / HOUR, (ms % HOUR) / MIN);
        return if m > 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        };
    }
    let (d, h) = (ms / DAY, (ms % DAY) / HOUR);
    if h > 0 {
        format!("{d}d{h}h")
    } else {
        format!("{d}d")
    }
}

/// 默认折叠阈值（行）。结果超出就折叠。
const DEFAULT_FOLD: usize = 5;
/// diff 类工具放宽：一屏 diff 值得直接看。
const DIFF_FOLD: usize = 14;
/// 网页三件套（fetch / browser / search）：内容是文档或结果清单，收起时看头 3 行。
const WEB_FOLD: usize = 3;
/// 上面那三个展开后给到 12 行（omp 的 `previewLimit`）。
const WEB_OPEN: usize = 12;

/// 带标签的正文段落。
pub(super) struct Section {
    pub label: Option<&'static str>,
    pub lines: Vec<Line<'static>>,
}

/// 工具块的生命周期状态 —— 决定字形、边框色、底色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    /// 还没结果（等工具跑完）。
    Pending,
    /// 成功。
    Ok,
    /// 失败。
    Err,
}

impl State {
    fn glyph(self) -> &'static str {
        match self {
            State::Pending => g::PENDING,
            State::Ok => g::OK,
            State::Err => g::ERR,
        }
    }

    /// 字形色：成功绿、失败红、进行中强调色。
    fn glyph_token(self) -> Token {
        match self {
            State::Pending => Token::ToolEdgePending,
            State::Ok => Token::Ok,
            State::Err => Token::Err,
        }
    }

    /// 边框色。**成功是 dim 灰**（omp 的规矩）：成功不该抢眼，红只留给失败。
    fn edge_token(self) -> Token {
        match self {
            State::Pending => Token::ToolEdgePending,
            State::Ok => Token::ToolEdgeSuccess,
            State::Err => Token::ToolEdgeError,
        }
    }

    /// 整块的底色。
    fn bg_token(self) -> Token {
        match self {
            State::Pending => Token::ToolBgPending,
            State::Ok => Token::ToolBgSuccess,
            State::Err => Token::ToolBgError,
        }
    }
}

/// 一个工具在转录里的样子：折叠策略、header 里的事实、调用预览、结果段落。
///
/// **一个工具 = 一个 impl + 一行注册**（见 [`renderer_for`]）。加工具不再往
/// 九个 `match name` 里各塞一个分支，也就不存在"漏改一处、静默降级"的路径。
/// omp 的 `packages/tui/src/tools/renderer.ts` 是同一个形状：一个
/// `ToolRenderer` 接口 + 一张 `toolRenderers` 表 + 一个兜底渲染器。
///
/// 渲染器只认数据（`args` 是模型给的原始 JSON，`details` 是工具交的结构化
/// 载荷），不认工具的实现——这是"换 UI 不用重写工具"的那条缝。
pub(super) trait ToolRenderer {
    /// 结果超过几行就折叠。
    fn fold_limit(&self) -> usize {
        DEFAULT_FOLD
    }

    /// 折叠丢掉哪一头：命令输出留尾（报错在最后），文档与文件留头。
    fn fold_takes_head(&self) -> bool {
        false
    }

    /// header 里 `·` 连接的零散事实：操作对象（路径 / URL / 查询词）。
    /// 「结果规模」不在这里——它是所有工具共用的尾部，由 [`ToolBlock::meta`] 补。
    fn meta(&self, _b: &ToolBlock<'_>) -> Vec<String> {
        Vec::new()
    }

    /// 结果规模的量词（"12 lines" / "8 results"）。默认按非空行数算。
    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let lines = non_empty_lines(o.text);
        (lines > 1).then(|| format!("{lines} lines"))
    }

    /// 调用正文（无标签的那一段）。默认：参数转 JSON。
    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        default_call(b.name, b.args, t)
    }

    /// 结果段落。默认一段：普通输出，按 [`Self::fold_limit`] 折叠。
    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        let lines = if b.name == "read" {
            read_body(o, t)
        } else {
            output_lines(o.text, self.fold_limit(), b.expanded, self.fold_takes_head(), t)
        };
        vec![Section { label: None, lines }]
    }

    /// 结果是不是 diff 形状（工具用 `details` 声明；diff 永远不折叠）。
    fn is_diff(&self, o: ToolOutcome<'_>) -> bool {
        ToolView::from_details(o.details, o.text).is_diff()
    }
}

/// 工具名 → 渲染器。认不出的名字落 [`Fallback`]：新工具先有兜底，再有画法。
pub(super) fn renderer_for(name: &str) -> &'static dyn ToolRenderer {
    match name {
        "bash" => &Bash,
        "edit" | "mass_edit" => &Edit,
        "read" | "write" | "cd" => &FileTools,
        "todo" => &Todo,
        "fetch" | "search" | "browser" => &Web,
        _ => &Fallback,
    }
}

/// bash：`$ 命令`，命令走 shell 高亮；结果按普通输出折叠（留尾，报错在最后）。
struct Bash;

impl ToolRenderer for Bash {
    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        bash_call(b.args, t)
    }
}

/// edit / mass_edit：调用预览是 `- old` / `+ new`，结果是同一件事的 diff，
/// 所以**结果到手就不画预览**（omp 的 `mergeCallAndResult`）。
struct Edit;

impl ToolRenderer for Edit {
    fn fold_limit(&self) -> usize {
        DIFF_FOLD
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        path_meta(b)
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        edit_call(b, t)
    }

    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        vec![Section {
            label: None,
            lines: edit_result(o, b.expanded, t),
        }]
    }
}

/// 文件类：read / write / cd。路径进 header 的 meta，正文只在 write 上有东西
/// （写进去的内容本身才是重点）。
struct FileTools;

impl ToolRenderer for FileTools {
    fn fold_takes_head(&self) -> bool {
        true
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = path_meta(b);
        // read 的续读区间：`path:10-19` 比 `path` 有用——它同时说明了这次看的
        // 是哪一段，和上次停在哪。
        if b.name == "read"
            && let Some(o) = b.result
            && let Some(d) = o.details
        {
            let offset = d.get("offset").and_then(|v| v.as_u64()).unwrap_or(1);
            let lines = d.get("lines").and_then(|v| v.as_u64()).unwrap_or(0);
            if offset > 1 && lines > 0 {
                let span = format!(":{}-{}", offset, offset + lines - 1);
                if let Some(first) = out.first_mut() {
                    first.push_str(&span);
                }
            }
        }
        out
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        // 文件工具的结果是内容本身，行数就是它的规模（read 的正文、write 的
        // "已覆盖…" 一行不算规模）。
        (self.fold_takes_head() && non_empty_lines(o.text) > 1)
            .then(|| format!("{} lines", non_empty_lines(o.text)))
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        if b.name == "write" {
            write_preview(b, t)
        } else {
            // 路径在 header 的 meta 里，正文不再重复。
            Vec::new()
        }
    }
}

/// fetch / search / browser：网页三件套。结果取头（开头才是正题），fetch 与
/// search 按 omp 分成「元信息 + 正文」两段。
struct Web;

impl ToolRenderer for Web {
    fn fold_limit(&self) -> usize {
        WEB_FOLD
    }

    fn fold_takes_head(&self) -> bool {
        true
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let mut out = Vec::new();
        match b.name {
            "fetch" => {
                if let Some(u) = arg_str(b.args, "url").filter(|u| !u.trim().is_empty()) {
                    out.push(short_url(&u));
                }
                if arg_bool(b.args, "raw") == Some(true) {
                    out.push("raw".into());
                }
            }
            "search" => {
                if let Some(q) = arg_str(b.args, "query").filter(|q| !q.trim().is_empty()) {
                    out.push(truncate(&one_line(&q), 60));
                }
            }
            "browser" => {
                if let Some(c) = arg_str(b.args, "command").filter(|c| !c.trim().is_empty()) {
                    out.push(one_line(&c));
                }
                // 具体的操作对象：op / url / path / selector，取第一个有的。
                for key in ["op", "url", "path", "selector"] {
                    if let Some(v) = arg_str(b.args, key).filter(|v| !v.trim().is_empty()) {
                        out.push(truncate(&one_line(&v), 50));
                        break;
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        // 搜索的量词是条数，不是行数 —— 「8 results」比「24 lines」有用。
        let hits = search_hits(o.text);
        if hits > 0 {
            return Some(format!("{hits} results"));
        }
        (non_empty_lines(o.text) > 1).then(|| format!("{} lines", non_empty_lines(o.text)))
    }

    fn call_body(&self, _b: &ToolBlock<'_>, _t: &HistoryTheme) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn result_sections(
        &self,
        b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        match b.name {
            "fetch" => fetch_sections(b, o.text, t),
            "search" => search_sections(b, o.text, t),
            _ => vec![Section {
                label: None,
                lines: head_preview(&non_empty_text_lines(o.text), WEB_FOLD, WEB_OPEN, b.expanded, t),
            }],
        }
    }
}

/// `todo`：模型的任务清单。
///
/// 它画的是**清单本身**，不是一次调用的流水账——这份清单是模型的备忘录，
/// 每次操作的结果都带全量（`details.phases`），所以卡片里永远能看到当前全貌。
/// 状态字形与颜色分开：`[x]` 绿、`[>]` 强调、`[!]` 警告、`[ ]`/`[-]` 弱化。
struct Todo;

impl ToolRenderer for Todo {
    fn fold_limit(&self) -> usize {
        // 清单是"看一眼全貌"的东西，别轻易折。
        DIFF_FOLD
    }

    fn meta(&self, b: &ToolBlock<'_>) -> Vec<String> {
        let op = arg_str(b.args, "op").unwrap_or_default();
        (!op.is_empty()).then_some(op).into_iter().collect()
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let d = o.details?;
        let (done, total) = (
            d.get("done").and_then(|v| v.as_u64())?,
            d.get("total").and_then(|v| v.as_u64())?,
        );
        Some(format!("{done}/{total} done"))
    }

    fn call_body(&self, b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
        // 结果到手就不画预览（结果带的是全量清单，预览只会重复）。
        if b.result.is_some() {
            return Vec::new();
        }
        // 还没有结果：至少让用户看见这一下想动哪一项（`init` 的全量清单
        // 等结果回来再画，那一侧才有权威状态）。
        arg_str(b.args, "task")
            .map(|task| vec![Line::from(t.fg(format!("→ {task}"), Token::Muted))])
            .unwrap_or_default()
    }

    fn result_sections(
        &self,
        _b: &ToolBlock<'_>,
        o: ToolOutcome<'_>,
        t: &HistoryTheme,
    ) -> Vec<Section> {
        let Some(d) = o.details else {
            return Vec::new();
        };
        let Some(phases) = d.get("phases").and_then(|p| p.as_array()) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for phase in phases {
            let name = phase.get("name").and_then(|n| n.as_str()).unwrap_or("");
            lines.push(Line::from(t.fg(name.to_string(), Token::ToolTitle)));
            for task in phase
                .get("tasks")
                .and_then(|t| t.as_array())
                .into_iter()
                .flatten()
            {
                lines.push(todo_line(task, t));
            }
        }
        if lines.is_empty() {
            return Vec::new();
        }
        vec![Section {
            label: None,
            lines,
        }]
    }
}

/// 清单里的一行：`[x] 内容（卡住：原因）`。
fn todo_line(task: &serde_json::Value, t: &HistoryTheme) -> Line<'static> {
    let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
    let content = task.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let (glyph, tok) = match status {
        "done" => (g::TODO_DONE, Token::Ok),
        "in_progress" => (g::TODO_ACTIVE, Token::ToolEdgePending),
        "blocked" => (g::TODO_BLOCKED, Token::Warn),
        "abandoned" => (g::TODO_DROPPED, Token::Dim),
        _ => (g::TODO_PENDING, Token::Dim),
    };
    // 做完的项整行弱化：它已经不需要注意力了。
    let body = if status == "done" {
        Token::Dim
    } else {
        Token::AssistantText
    };
    let mut spans = vec![
        t.fg(format!("{glyph} "), tok),
        t.fg(content.to_string(), body),
    ];
    if let Some(reason) = task.get("blocker").and_then(|b| b.as_str()) {
        spans.push(t.fg(format!("（卡住：{reason}）"), Token::Warn));
    }
    Line::from(spans)
}

/// 兜底：参数转 JSON（展开时全文，收起时一行），结果当普通输出。
struct Fallback;

impl ToolRenderer for Fallback {}

/// header 里的路径事实（read / write / cd / edit / mass_edit 共用）。
fn path_meta(b: &ToolBlock<'_>) -> Vec<String> {
    arg_str(b.args, "path")
        .filter(|p| !p.trim().is_empty())
        .map(|p| vec![one_line(&p)])
        .unwrap_or_default()
}

/// write 的调用预览：写进去的内容，带行号栏 + 按目标语言高亮，超行折叠。
///
/// 照抄 omp `tui/src/tools/write.ts` 的 `renderContentPreview`：行号栏 +
/// 内容 + 折叠提示。写入的正文就是这次调用的全部内容，把它画出来比画一行
/// JSON 参数有用得多。
fn write_preview(b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let content = arg_str(b.args, "content").unwrap_or_default();
    if content.is_empty() {
        return Vec::new();
    }
    let lang = highlight::language_for_path(&arg_str(b.args, "path").unwrap_or_default());
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let shown = if b.expanded {
        total
    } else {
        total.min(WRITE_PREVIEW_LINES)
    };
    let gutter = total.to_string().len().max(WRITE_GUTTER_MIN_WIDTH);
    let mut out = Vec::new();
    for (i, line) in lines.iter().take(shown).enumerate() {
        let mut spans = vec![t.fg(format!("{:>gutter$} ", i + 1), Token::Dim)];
        let hl = highlight::highlight(line, lang, t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(*line)),
        }
        out.push(Line::from(spans));
    }
    if total > shown {
        out.push(Line::from(t.fg(
            format!("… {} more lines", total - shown),
            Token::Dim,
        )));
    }
    out
}

/// write 的调用预览画几行（收起时）。够看出写的是什么，不占满屏。
const WRITE_PREVIEW_LINES: usize = 8;

/// 行号栏的最小宽度（omp 的 `WRITE_GUTTER_MIN_WIDTH`）：两行的文件也留出
/// `  1│` 的宽度，缩进不随内容长度跳。
const WRITE_GUTTER_MIN_WIDTH: usize = 3;

/// read 的结果：把正文里 `   12<TAB>内容` 的行号前缀重画成行号栏 `12│内容`。
///
/// 给**模型**的文本保持原样（行号 + 制表符，那是它对不齐时的依据）；这一层
/// 只管画得像 omp：制表符在卡片里会打乱对齐，行号该是一个独立的栏
/// （omp 的 `formatCodeFrameLine`）。认不出来的行（末尾那句「还有更多行」）
/// 按 dim 原样画。
fn read_body(o: ToolOutcome<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut numbered: Vec<(usize, String)> = Vec::new();
    for line in o.text.lines() {
        let parsed = line
            .trim_start()
            .split_once('\t')
            .and_then(|(n, text)| n.parse::<usize>().ok().map(|n| (n, text.replace('\t', "    "))));
        match parsed {
            Some((n, text)) => numbered.push((n, text)),
            None => {
                flush_numbered(&mut numbered, &mut out, t);
                out.push(Line::from(t.fg(line.to_string(), Token::Muted)));
            }
        }
    }
    flush_numbered(&mut numbered, &mut out, t);
    out
}

/// 一栏行号：宽度按本段最大的行号（不小于 `WRITE_GUTTER_MIN_WIDTH`）。
fn flush_numbered(
    numbered: &mut Vec<(usize, String)>,
    out: &mut Vec<Line<'static>>,
    t: &HistoryTheme,
) {
    if numbered.is_empty() {
        return;
    }
    let width = numbered
        .iter()
        .map(|(n, _)| n.to_string().len())
        .max()
        .unwrap_or(1)
        .max(WRITE_GUTTER_MIN_WIDTH);
    for (n, text) in numbered.drain(..) {
        let mut spans = vec![t.fg(format!("{n:>width$}│"), Token::Dim)];
        let hl = highlight::highlight(&text, None, t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(text)),
        }
        out.push(Line::from(spans));
    }
}

/// 一个工具块：调用参数 +（可选）结果。
#[derive(Debug, Clone, Copy)]
pub(super) struct ToolBlock<'a> {
    pub name: &'a str,
    pub args: &'a str,
    /// 结果（正文 + 成败 + 工具的 `details` + 耗时）。还没跑完就是 `None`。
    pub result: Option<ToolOutcome<'a>>,
    /// Ctrl+O：收起时结果取尾，展开时全给。
    pub expanded: bool,
}

impl<'a> ToolBlock<'a> {
    pub(super) fn state(&self) -> State {
        match self.result {
            None => State::Pending,
            Some(o) if o.ok => State::Ok,
            Some(_) => State::Err,
        }
    }

    pub(super) fn edge_style(&self, t: &HistoryTheme) -> Style {
        t.fg_style(self.state().edge_token())
    }

    pub(super) fn bg_style(&self, t: &HistoryTheme) -> Style {
        Style::new().bg(t.get(self.state().bg_token()))
    }

    /// header：`<字形> <工具名>: <intent> · <meta>`
    pub(super) fn header(&self, t: &HistoryTheme) -> Line<'static> {
        let state = self.state();
        let mut spans = vec![
            t.fg(state.glyph(), state.glyph_token()),
            t.fg_mod(
                format!(" {}", self.name),
                Token::ToolTitle,
                Modifier::BOLD,
            ),
        ];
        if let Some(d) = description(self.args) {
            spans.push(t.fg(format!(": {d}"), Token::Muted));
        }
        for m in self.meta() {
            spans.push(t.fg(format!(" {} {m}", g::SEP), Token::Dim));
        }
        Line::from(spans)
    }

    /// 这个工具的渲染器（注册表里那一行）。
    fn r(&self) -> &'static dyn ToolRenderer {
        renderer_for(self.name)
    }

    /// meta：零散小事实，`·` 连接 —— 操作对象（路径 / URL / 查询词）由渲染器
    /// 给，「结果规模」与「耗时」是所有工具共用的尾部，在这里补。
    fn meta(&self) -> Vec<String> {
        let mut out = self.r().meta(self);
        if let Some(o) = self.result {
            if let Some(m) = self.r().result_measure(o) {
                out.push(m);
            }
            // 耗时报在最后：它是"这次调用有多重"的补充，不是主要事实。
            if let Some(d) = o.duration_label() {
                out.push(d);
            }
        }
        out
    }

    /// 调用正文（无标签的那一段）。
    pub(super) fn call_body(&self, t: &HistoryTheme) -> Vec<Line<'static>> {
        self.r().call_body(self, t)
    }

    /// 整个块的段落序列：调用段在前，结果段在后。空段落不参与。
    pub(super) fn sections(&self, t: &HistoryTheme) -> Vec<Section> {
        let mut out = Vec::new();
        let call = self.call_body(t);
        if !call.is_empty() {
            out.push(Section {
                label: None,
                lines: call,
            });
        }
        if let Some(o) = self.result {
            out.extend(self.result_sections(o, t));
        }
        out.retain(|s| !s.lines.is_empty());
        out
    }

    /// 结果段落。多数工具一段；fetch / search 按 omp 分成「元信息 + 正文」。
    fn result_sections(&self, o: ToolOutcome<'_>, t: &HistoryTheme) -> Vec<Section> {
        self.r().result_sections(self, o, t)
    }

    /// Ctrl+O 是否改这个块的渲染（缓存据此决定存一份还是两份）。
    pub(super) fn has_two_states(&self) -> bool {
        let Some(o) = self.result else {
            return false;
        };
        if self.r().is_diff(o) {
            return false;
        }
        non_empty_lines(o.text) > self.r().fold_limit()
    }

}

// ---------------------------------------------------------------------------
// header 取词
// ---------------------------------------------------------------------------

/// 模型必填的一句话说明（`intent`）—— 这次调用要干什么。它才是给人看的
/// description；命令、路径都在正文里，不在这里重复。
fn description(args: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    let intent = v.get("intent")?.as_str()?.trim();
    if intent.is_empty() {
        return None;
    }
    Some(one_line(intent))
}

/// header 只能占一行：把片段里的换行压成空格，否则会把框撑破。
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

// ---------------------------------------------------------------------------
// 正文画法
// ---------------------------------------------------------------------------

/// bash：`$ <命令>`，命令过 shell 高亮。
fn bash_call(args: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    let cmd = arg_str(args, "command").unwrap_or_default();
    if cmd.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, line) in cmd.lines().enumerate() {
        let mut spans = Vec::new();
        // `$ ` 前缀只加第一行：多行命令每行独立起色，前缀加满会读成
        // 每一行都是一条命令。
        if i == 0 {
            spans.push(t.fg("$ ", Token::Dim));
        }
        let hl = highlight::highlight(line, Some("sh"), t);
        match hl.into_iter().next() {
            Some(l) if !l.spans.is_empty() => spans.extend(l.spans),
            _ => spans.push(t.plain(line)),
        }
        out.push(Line::from(spans));
    }
    out
}

/// edit / mass_edit：`- old` / `+ new`，按目标文件的语言高亮。
///
/// 结果到手后就不再画一遍 —— 结果的 diff 说的正是同一件事（omp 的
/// `mergeCallAndResult`）。只有还没跑完时才靠这层预览看要改什么。
fn edit_call(b: &ToolBlock<'_>, t: &HistoryTheme) -> Vec<Line<'static>> {
    if b.result.is_some() {
        return Vec::new();
    }
    let path = arg_str(b.args, "path").unwrap_or_default();
    let lang = highlight::language_for_path(&path);
    let mut out = Vec::new();
    if let Some(old) = arg_str(b.args, "old") {
        push_diff(&mut out, '-', &old, lang, t);
    }
    if let Some(new) = arg_str(b.args, "new") {
        push_diff(&mut out, '+', &new, lang, t);
    }
    // mass_edit 说的是行号而不是原文：逐行点名要换掉哪几行。
    if out.is_empty()
        && let Some(edits) = serde_json::from_str::<serde_json::Value>(b.args)
            .ok()
            .and_then(|v| v.get("edits").and_then(|e| e.as_array()).cloned())
    {
        let mut text = String::new();
        for e in edits {
            let line = e.get("line").and_then(|l| l.as_i64()).unwrap_or(0);
            let body = e.get("text").and_then(|l| l.as_str()).unwrap_or("");
            text.push_str(&format!("{line}: {body}\n"));
        }
        push_diff(&mut out, '+', text.trim_end(), lang, t);
    }
    out
}

/// 一行 diff：`<符号> <内容>`，符号与内容同色，内容照旧走语法高亮。
fn push_diff(
    out: &mut Vec<Line<'static>>,
    sign: char,
    text: &str,
    lang: Option<&str>,
    t: &HistoryTheme,
) {
    let tok = if sign == '-' {
        Token::DiffRemoved
    } else {
        Token::DiffAdded
    };
    for l in text.lines() {
        let mut spans = vec![t.fg(format!("{sign} "), tok)];
        for hl in highlight::highlight(l, lang, t) {
            spans.extend(hl.spans);
        }
        out.push(Line::from(spans));
    }
}


/// edit / mass_edit 的结果：diff 就画 red/green 行，否则当普通输出。
fn edit_result(o: ToolOutcome<'_>, expanded: bool, t: &HistoryTheme) -> Vec<Line<'static>> {
    match ToolView::from_details(o.details, o.text) {
        ToolView::Diff {
            deletions,
            insertions,
            at_line,
        } => {
            let card = t.get(Token::ToolBgSuccess);
            let fg = contrast_on(card);
            // 行号栏的宽度按**最大的那个行号**算：`-315│` / `   +│`，
            // 标记列与行号列一起右对齐（omp 的 `formatCodeFrameLine`）。
            let numbers = at_line.map(|at| {
                let last = at + deletions.len().max(insertions.len()).saturating_sub(1);
                last.to_string().len()
            });
            // 标记列与行号列**一起**右对齐（omp：`gutterText.padStart(width + 1)`），
            // 所以去重后留下的空行号是 `  +│`——标记落到最后一列，不再错位。
            let gutter = |sign: char, idx: usize, blank_number: bool| -> String {
                match (at_line, numbers) {
                    (Some(at), Some(w)) => {
                        let text = if blank_number {
                            sign.to_string()
                        } else {
                            format!("{sign}{}", at + idx)
                        };
                        format!("{text:>pad$}│", pad = w + 1)
                    }
                    // 工具没报行号：只留标记列，绝不编一个数出来。
                    _ => format!("{sign} "),
                }
            };
            let mut out = Vec::new();
            // 行号去重：**与上一行相同**的那个留空（omp 的规矩，例子是
            // 单行替换 `-12` / `+12` 与「插入 + 紧邻上下文」）。
            // 用"相同才留空"而不是"两边等长就全留空"——2:2 的替换里
            // `+98` 的前一行是 `-99`，本来就该印出来。
            let mut last_shown: Option<usize> = None;
            let mut emit = |sign: char, idx: usize, text: &str, tok: Token| {
                let n = at_line.map(|at| at + idx);
                let blank = matches!((n, last_shown), (Some(a), Some(b)) if a == b);
                last_shown = n;
                out.push(Line::styled(
                    format!("{}{text}", gutter(sign, idx, blank)),
                    Style::new().fg(fg).bg(dim_bg(t.get(tok), card)),
                ));
            };
            for (i, d) in deletions.iter().enumerate() {
                emit('-', i, d, Token::DiffRemoved);
            }
            for (i, ins) in insertions.iter().enumerate() {
                emit('+', i, ins, Token::DiffAdded);
            }
            out
        }
        ToolView::Plain { text } => output_lines(&text, DIFF_FOLD, expanded, false, t),
    }
}

/// 普通输出：超阈值就折叠，挂一行折叠标记 + 展开提示。
///
/// 丢哪一头由 [`fold_takes_head`] 定：命令输出留尾（报错在最后），文件
/// 与文档留头。
pub(super) fn output_lines(
    text: &str,
    limit: usize,
    expanded: bool,
    takes_head: bool,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        // 跑了但没输出，仍然发生过：占位一行，别让卡片看起来像被跳过。
        return vec![Line::from(t.fg("(no output)", Token::Muted))];
    }
    let lines: Vec<&str> = text.lines().collect();
    let fold = !expanded && lines.len() > limit;
    let hidden = lines.len().saturating_sub(limit);
    let marker = if takes_head {
        format!("… {hidden} more lines {}", g::EXPAND_HINT)
    } else {
        format!("… {hidden} earlier lines {}", g::EXPAND_HINT)
    };
    let kept: &[&str] = if !fold {
        &lines[..]
    } else if takes_head {
        &lines[..limit]
    } else {
        &lines[hidden..]
    };
    let mut out = Vec::new();
    if fold && takes_head {
        // 取头时标记垫在内容后面，读起来才是「下面还有」。
        for l in kept {
            out.push(Line::from(t.fg(*l, Token::ToolOutput)));
        }
        out.push(Line::from(t.fg(marker, Token::Dim)));
        return out;
    }
    if fold {
        out.push(Line::from(t.fg(marker, Token::Dim)));
    }
    for l in kept {
        out.push(Line::from(t.fg(*l, Token::ToolOutput)));
    }
    out
}

/// 取头的预览：文档、结果清单都从第一行读起。
///
/// 收起给 `collapsed` 行，展开给 `cap` 行，超出的部分用一行标记交代。
/// （omp `fetch` / `web-search` 的 `previewLimit` 就是这个形状。）
fn head_preview(
    lines: &[String],
    collapsed: usize,
    cap: usize,
    expanded: bool,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let limit = if expanded { cap } else { collapsed };
    if lines.is_empty() {
        return vec![Line::from(t.fg("(no content)", Token::Muted))];
    }
    let mut out: Vec<Line<'static>> = lines
        .iter()
        .take(limit)
        .map(|l| Line::from(t.fg(l.clone(), Token::ToolOutput)))
        .collect();
    if lines.len() > limit {
        out.push(Line::from(t.fg(
            format!(
                "… {} more lines {}",
                lines.len() - limit,
                g::EXPAND_HINT
            ),
            Token::Dim,
        )));
    }
    out
}

/// fetch：`metadata`（URL / 模式 / 行数 / 字符数）+ `content`（正文取头）。
/// 照抄 omp `tools/fetch.ts` 的两段式卡片。
fn fetch_sections(b: &ToolBlock<'_>, text: &str, t: &HistoryTheme) -> Vec<Section> {
    let url = arg_str(b.args, "url").unwrap_or_default();
    let raw = arg_bool(b.args, "raw") == Some(true);
    let lines = non_empty_text_lines(text);
    let chars = text.trim().chars().count();

    let mut meta: Vec<Line<'static>> = Vec::new();
    if !url.trim().is_empty() {
        meta.push(fact("URL: ", &truncate(&one_line(&url), 80), t));
    }
    meta.push(fact(
        "Mode: ",
        if raw { "raw HTML" } else { "markdown" },
        t,
    ));
    meta.push(fact("Lines: ", &lines.len().to_string(), t));
    meta.push(fact("Chars: ", &chars.to_string(), t));

    vec![
        Section {
            label: Some("metadata"),
            lines: meta,
        },
        Section {
            label: Some("content"),
            lines: head_preview(&lines, WEB_FOLD, WEB_OPEN, b.expanded, t),
        },
    ]
}

/// search：`Query` 一行 + `results`（结果清单取头）。照抄 omp
/// `tools/web-search.ts` 的 header + Answer/Sources 分层。
fn search_sections(b: &ToolBlock<'_>, text: &str, t: &HistoryTheme) -> Vec<Section> {
    let query = arg_str(b.args, "query").unwrap_or_default();
    let lines = non_empty_text_lines(text);
    let mut out = Vec::new();
    if !query.trim().is_empty() {
        out.push(Section {
            label: None,
            lines: vec![fact("Query: ", &one_line(&query), t)],
        });
    }
    out.push(Section {
        label: Some("results"),
        lines: head_preview(&lines, WEB_FOLD, WEB_OPEN, b.expanded, t),
    });
    out
}

/// 一条「标签 + 值」的元信息行：标签弱化，值正常。
fn fact(label: &str, value: &str, t: &HistoryTheme) -> Line<'static> {
    Line::from(vec![
        t.fg(label, Token::Muted),
        t.fg(value.to_string(), Token::ToolOutput),
    ])
}

/// 兜底：参数转 JSON（展开时全文，收起时一行），结果当普通输出。
fn default_call(name: &str, args: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    let lang = highlight::language_for_tool(name);
    match serde_json::from_str::<serde_json::Value>(args) {
        Ok(v) => {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args.to_string());
            let mut out = Vec::new();
            for l in pretty.lines() {
                out.extend(highlight::highlight(l, Some("json"), t));
            }
            out
        }
        Err(_) => {
            let mut out = Vec::new();
            for l in args.lines() {
                out.extend(highlight::highlight(l, lang, t));
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// 结果里的非空行数 —— 空行撑不出信息，别计进「N lines」。
fn non_empty_lines(text: &str) -> usize {
    non_empty_text_lines(text).len()
}

fn non_empty_text_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

/// 搜索结果条数：几行是「N. 标题」就有几条。
fn search_hits(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            let l = l.trim_start();
            let digits: String = l.chars().take_while(char::is_ascii_digit).collect();
            !digits.is_empty() && l[digits.len()..].starts_with(". ")
        })
        .count()
}

/// 只取 URL 的「主机 + 路径」，省掉 scheme，长了就截。
fn short_url(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    truncate(&one_line(rest), 60)
}

/// 按显示宽度截断（header / meta 只有一行可占）。
fn truncate(text: &str, max: usize) -> String {
    let (cut, w) = crate::tui::text::take_width(text, max);
    if w < unicode_width::UnicodeWidthStr::width(text) {
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

fn arg_bool(args: &str, key: &str) -> Option<bool> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    v.get(key)?.as_bool()
}

fn arg_str(args: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args).ok()?;
    Some(v.get(key)?.as_str()?.to_string())
}

impl ToolView {
    /// 是不是 diff 形状（决定用哪套画法、以及缓存要不要存两份）。
    pub(super) fn is_diff(&self) -> bool {
        matches!(self, ToolView::Diff { .. })
    }
}

/// 卡片底色上可读的前景色（omp 的对比度规矩）。
fn contrast_on(bg: Color) -> Color {
    crate::tui::theme::contrast_text_on(bg)
}

/// diff 行底色：diff 色朝卡片底色压淡，整行铺满也不刺眼。
fn dim_bg(fg: Color, card: Color) -> Color {
    let (fr, fg_, fb) = match fg {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    let (cr, cg, cb) = match card {
        Color::Rgb(r, g, b) => (r as u32, g as u32, b as u32),
        _ => return card,
    };
    Color::Rgb(
        ((cr * 3 + fr) / 4) as u8,
        ((cg * 3 + fg_) / 4) as u8,
        ((cb * 3 + fb) / 4) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    /// 段落拼回一段文本，测试里只关心内容。
    fn sections_text(b: &ToolBlock, t: &HistoryTheme) -> Vec<Line<'static>> {
        b.sections(t).into_iter().flat_map(|s| s.lines).collect()
    }

    fn text_of(lines: &[Line]) -> String {
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

    /// header 四段：字形 + 工具名 + intent + meta，全在一行里。
    #[test]
    fn header_carries_glyph_name_intent_and_meta() {
        let t = t();
        let b = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑测试","command":"npm test"}"#,
            result: Some(ToolOutcome::text("a\nb\nc", true)),
            expanded: false,
        };
        let h = text_of(&[b.header(&t)]);
        assert!(h.contains(g::OK), "缺状态字形：{h}");
        assert!(h.contains("bash"), "缺工具名：{h}");
        assert!(h.contains("跑测试"), "缺 intent：{h}");
        assert!(h.contains("3 lines"), "缺 meta：{h}");
        assert!(!h.contains('\n'), "header 撑成了多行：{h}");
    }

    /// 还没结果 → 进行中字形、强调色边框。
    #[test]
    fn pending_block_is_not_dressed_as_success() {
        let t = t();
        let b = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑测试","command":"sleep 1"}"#,
            result: None,
            expanded: false,
        };
        assert_eq!(b.state(), State::Pending);
        let h = text_of(&[b.header(&t)]);
        assert!(h.contains(g::PENDING), "进行中没有自己的字形：{h}");
        assert!(!h.contains(g::OK), "还没跑完就画成功：{h}");
        assert_eq!(
            b.edge_style(&t).fg,
            Some(t.get(Token::ToolEdgePending)),
            "进行中的边框不是强调色"
        );
    }

    /// 成功与失败一眼可辨：字形、边框都不同。
    #[test]
    fn success_and_failure_do_not_look_alike() {
        let t = t();
        let ok = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑测试","command":"npm test"}"#,
            result: Some(ToolOutcome::text("PASS", true)),
            expanded: false,
        };
        let bad = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑测试","command":"npm test"}"#,
            result: Some(ToolOutcome::text("boom", false)),
            expanded: false,
        };
        assert_eq!(ok.state(), State::Ok);
        assert_eq!(bad.state(), State::Err);
        assert_ne!(text_of(&[ok.header(&t)]), text_of(&[bad.header(&t)]));
        assert_ne!(ok.edge_style(&t).fg, bad.edge_style(&t).fg);
    }

    /// 折叠：取尾，前面挂隐藏行数 + 展开提示；展开后是全文。
    #[test]
    fn folding_keeps_the_tail_and_says_how_much_is_hidden() {
        let t = t();
        let body: String = (1..=12).map(|i| format!("line {i}\n")).collect();
        let b = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑","command":"x"}"#,
            result: Some(ToolOutcome::text(body.as_str(), true)),
            expanded: false,
        };
        let folded = text_of(&sections_text(&b, &t));
        assert!(folded.contains("7 earlier lines"), "没报隐藏行数：{folded}");
        assert!(folded.contains(g::EXPAND_HINT), "没有展开提示：{folded}");
        assert!(!folded.contains("line 1\n"), "取尾却还留着头部：{folded}");
        assert!(folded.contains("line 12"), "尾没保住：{folded}");

        let open = ToolBlock {
            expanded: true,
            ..b
        };
        let full = text_of(&sections_text(&open, &t));
        assert!(full.contains("line 1"), "展开后没给全文：{full}");
        assert!(!full.contains(g::EXPAND_HINT), "展开了还提示展开：{full}");
    }

    #[test]
    fn fold_switch_matters_only_when_lines_are_hidden() {
        let short = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑","command":"x"}"#,
            result: Some(ToolOutcome::text("a\nb", true)),
            expanded: false,
        };
        assert!(!short.has_two_states(), "没超阈值却说要两份渲染");
        let long = ToolBlock {
            result: Some(ToolOutcome::text("a\nb\nc\nd\ne\nf", true)),
            ..short
        };
        assert!(long.has_two_states());
    }

    /// fetch：`metadata` + `content` 两段，正文取头（正文不是命令输出，
    /// 开头才是正题）。照 omp `tools/fetch.ts`。
    #[test]
    fn fetch_splits_metadata_from_a_head_preview() {
        let t = t();
        let body: String = (1..=9).map(|i| format!("line {i}\n")).collect();
        let b = ToolBlock {
            name: "fetch",
            args: r#"{"intent":"读文档","url":"https://x.dev/a"}"#,
            result: Some(ToolOutcome::text(body.as_str(), true)),
            expanded: false,
        };
        let sections = b.sections(&t);
        let labels: Vec<&str> = sections.iter().filter_map(|s| s.label).collect();
        assert_eq!(labels, vec!["metadata", "content"], "段落分层变了");
        let all = text_of(&sections.into_iter().flat_map(|s| s.lines).collect::<Vec<_>>());
        assert!(all.contains("URL: https://x.dev/a"), "{all}");
        assert!(all.contains("Mode: markdown"), "没报抓取模式: {all}");
        assert!(all.contains("line 1") && !all.contains("line 9"), "该取头: {all}");
        assert!(all.contains("… 6 more lines"), "{all}");
    }

    /// search：结果条数是「几条」，不是「几行」—— 别把 url/摘要行也数成结果。
    #[test]
    fn search_counts_results_not_lines() {
        let t = t();
        let body = "1. axum 例子\n   https://g.com/a\n   官方示例\n\n2. 文档\n   https://d.rs/b\n";
        let b = ToolBlock {
            name: "search",
            args: r#"{"intent":"找例子","query":"axum middleware"}"#,
            result: Some(ToolOutcome::text(body, true)),
            expanded: false,
        };
        let head = text_of(&[b.header(&t)]);
        assert!(head.contains("2 results"), "结果数不对: {head}");
        assert!(!head.contains("lines"), "不该按行数报: {head}");
        let all = text_of(&b.sections(&t).into_iter().flat_map(|s| s.lines).collect::<Vec<_>>());
        assert!(all.contains("Query: axum middleware"), "查询词没露出来: {all}");
    }

    /// edit：结果到手后不再重复画一遍调用预览（两处说的是同一件事）。
    #[test]
    fn edit_preview_yields_to_the_result() {
        let t = t();
        let args = r#"{"intent":"改","path":"m.rs","old":"let a","new":"let b"}"#;
        let fresh = ToolBlock {
            name: "edit",
            args,
            result: None,
            expanded: false,
        };
        let preview = text_of(&fresh.call_body(&t));
        assert!(
            preview.contains("- let a") && preview.contains("+ let b"),
            "还没结果时该预览要改什么: {preview}"
        );
        let landed = ToolBlock {
            result: Some(ToolOutcome::text("+ let b", true)),
            ..fresh
        };
        assert!(
            landed.call_body(&t).is_empty(),
            "结果已经在画 diff 了，调用预览不该再来一遍"
        );
    }

    /// bash 的命令行只加一次 `$ ` 前缀。
    #[test]
    fn bash_prefix_is_printed_once() {
        let t = t();
        let b = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑","command":"echo a\necho b"}"#,
            result: None,
            expanded: false,
        };
        let body = text_of(&b.call_body(&t));
        assert_eq!(body.matches("$ ").count(), 1, "前缀加了不止一次：{body}");
        assert!(body.contains("echo b"));
    }

    #[test]
    fn the_todo_card_draws_the_whole_list_with_a_glyph_per_status() {
        // 卡片画的是**清单本身**（模型的备忘录），所以五个状态都要有字形，
        // 卡住的项要带原因，header 的 meta 要有进度。
        let t = t();
        let details = serde_json::json!({
            "kind": "todo",
            "op": "done",
            "phases": [
                {"name": "阶段一", "tasks": [
                    {"content": "甲", "status": "done"},
                    {"content": "乙", "status": "in_progress"},
                    {"content": "丙", "status": "pending"},
                    {"content": "丁", "status": "blocked", "blocker": "等上游"},
                    {"content": "戊", "status": "abandoned"},
                ]},
            ],
            "done": 1,
            "total": 5,
        });
        let b = ToolBlock {
            name: "todo",
            args: r#"{"intent":"推进","op":"done","task":"甲"}"#,
            result: Some(ToolOutcome {
                text: "1/5 done",
                ok: true,
                details: Some(&details),
                duration_ms: 0,
            }),
            expanded: false,
        };
        let body = text_of(&sections_text(&b, &t));
        for needle in ["阶段一", "[x] 甲", "[>] 乙", "[ ] 丙", "[!] 丁", "[-] 戊", "等上游"] {
            assert!(body.contains(needle), "缺 {needle}：\n{body}");
        }
        let header = text_of(&[b.header(&t)]);
        assert!(header.contains("1/5 done"), "header 缺进度：{header}");
        assert!(header.contains("done"), "header 要说明这次是什么操作：{header}");
    }

    #[test]
    fn a_pending_todo_call_shows_which_item_it_touches() {
        // 结果还没回来时，至少让人看见这一下想动哪一项。
        let t = t();
        let b = ToolBlock {
            name: "todo",
            args: r#"{"intent":"推进","op":"start","task":"接 /switch"}"#,
            result: None,
            expanded: false,
        };
        let body = text_of(&b.call_body(&t));
        assert!(body.contains("接 /switch"), "{body}");
    }

    #[test]
    fn write_shows_the_content_it_is_writing_with_a_line_gutter() {
        // 写入的正文就是这次调用的全部内容：画出来比画一行 JSON 参数有用。
        // 形状照抄 omp 的 `renderContentPreview`（行号栏 + 内容 + 折叠提示）。
        let t = t();
        let args = serde_json::json!({
            "intent": "写配置",
            "path": "conf.toml",
            "content": "a = 1\nb = 2\n"
        })
        .to_string();
        let b = ToolBlock {
            name: "write",
            args: &args,
            result: None,
            expanded: false,
        };
        let body = text_of(&b.call_body(&t));
        assert!(body.contains("1 a = 1"), "缺行号栏: {body}");
        assert!(body.contains("2 b = 2"), "缺第二行: {body}");
        assert!(!body.contains("+ "), "预览不是 diff，不该带 +/- 前缀: {body}");
        assert!(!body.contains("content"), "不该把参数名画出来: {body}");
    }

    #[test]
    fn a_long_write_folds_to_a_preview_and_says_how_much_is_hidden() {
        let t = t();
        let content: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        let args = serde_json::json!({"intent": "写", "path": "f.txt", "content": content}).to_string();
        let collapsed = ToolBlock {
            name: "write",
            args: &args,
            result: None,
            expanded: false,
        };
        let body = text_of(&collapsed.call_body(&t));
        assert!(body.contains("… 12 more lines"), "收起时要说还剩多少行: {body}");
        assert!(!body.contains("line9"), "收起时不该画到第 9 行: {body}");

        // Ctrl+O 展开：全文。
        let expanded = ToolBlock {
            name: "write",
            args: &args,
            result: None,
            expanded: true,
        };
        let body = text_of(&expanded.call_body(&t));
        assert!(body.contains("20 line20"), "展开必须给全文: {body}");
        assert!(!body.contains("more lines"));
    }

    #[test]
    fn a_paged_read_says_which_range_it_is() {
        // 续读的 read 在 header 里带区间：它同时说明这次看的是哪一段、上次停在哪。
        let t = t();
        let details = serde_json::json!({"kind": "file", "path": "/x/big.txt", "lines": 10, "offset": 11, "more": true});
        let b = ToolBlock {
            name: "read",
            args: r#"{"intent":"读","path":"big.txt"}"#,
            result: Some(ToolOutcome {
                text: "…",
                ok: true,
                details: Some(&details),
                duration_ms: 0,
            }),
            expanded: false,
        };
        let header = text_of(&[b.header(&t)]);
        assert!(header.contains("big.txt:11-20"), "缺区间: {header}");
    }

    #[test]
    fn an_unknown_tool_falls_back_to_json_and_plain_output() {
        // 兜底渲染器：新工具先有兜底，再有画法。
        let t = t();
        let b = ToolBlock {
            name: "brand_new",
            args: r#"{"intent":"试试","x":1}"#,
            result: Some(ToolOutcome::text("out", true)),
            expanded: true,
        };
        let body = text_of(&sections_text(&b, &t));
        assert!(body.contains("\"x\""), "兜底把参数转 JSON: {body}");
        assert!(body.contains("out"), "兜底把结果当普通输出: {body}");
    }

    /// 渲染层要画行号栏：edit 的 diff 是 `12│`，read 的正文是 `12│`。
    ///
    /// omp 的 `formatCodeFrameLine` 就是这个形状（`<标记><行号>│内容`）。
    /// 这两条是**屏幕上能看见**的东西，所以断言画出来的文本。
    #[test]
    fn diffs_and_reads_draw_a_line_number_gutter() {
        let t = HistoryTheme::resolve();
        // edit：1:1 替换 —— 删除侧印行号，插入侧留空（omp 的去重）。
        let details = serde_json::json!({
            "kind": "diff",
            "deletions": ["旧的一行"],
            "insertions": ["新的一行"],
            "atLine": 12,
        });
        let o = ToolOutcome {
            text: "replaced: x.rs",
            ok: true,
            details: Some(&details),
            duration_ms: 0,
        };
        let got: Vec<String> = edit_result(o, false, &t)
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(got, vec!["-12│旧的一行", "  +│新的一行"], "edit 的行号栏");

        // 多行替换：两边行号各自递增。
        let many = serde_json::json!({
            "kind": "diff",
            "deletions": ["一", "二"],
            "insertions": ["一改", "二改"],
            "atLine": 98,
        });
        let got: Vec<String> = edit_result(
            ToolOutcome {
                text: "replaced: x.rs",
                ok: true,
                details: Some(&many),
                duration_ms: 0,
            },
            false,
            &t,
        )
        .iter()
        .map(|l| l.to_string())
        .collect();
        assert_eq!(got, vec!["-98│一", "-99│二", "+98│一改", "+99│二改"]);

        // read：正文的 `   12\t内容` 重画成 `12│内容`，末尾那句提示原样。
        let o = ToolOutcome {
            text: "   11\t第一行\n   12\t第二行\n\n[还有更多行；继续读用 offset=3]",
            ok: true,
            details: None,
            duration_ms: 0,
        };
        let got: Vec<String> = read_body(o, &t).iter().map(|l| l.to_string()).collect();
        assert_eq!(got[0], " 11│第一行", "行号右对齐到最小宽度 3");
        assert_eq!(got[1], " 12│第二行");
        assert!(got[3].contains("继续读"), "尾注原样画：{}", got[3]);
    }

    /// 老 payload（没有 `atLine`）不许凭空编行号。
    #[test]
    fn a_diff_without_line_numbers_only_draws_the_marker_column() {
        let t = HistoryTheme::resolve();
        let details = serde_json::json!({
            "kind": "diff",
            "deletions": ["a"],
            "insertions": ["b"],
        });
        let got: Vec<String> = edit_result(
            ToolOutcome {
                text: "",
                ok: true,
                details: Some(&details),
                duration_ms: 0,
            },
            false,
            &t,
        )
        .iter()
        .map(|l| l.to_string())
        .collect();
        assert_eq!(got, vec!["- a", "+ b"]);
    }

    #[test]
    fn a_diff_view_comes_from_the_tools_details() {
        // The tool declares the shape; the renderer obeys. No re-parsing of the
        // result sentence — "replaced: x.rs" carries no diff at all.
        let details = serde_json::json!({
            "kind": "diff",
            "deletions": ["old line"],
            "insertions": ["new line"],
            "atLine": 12,
        });
        let view = ToolView::from_details(Some(&details), "replaced: x.rs");
        assert_eq!(
            view,
            ToolView::Diff {
                deletions: vec!["old line".into()],
                insertions: vec!["new line".into()],
                at_line: Some(12),
            }
        );
        assert!(view.is_diff());
        // 老 payload（`atLine` 之前落的库）照样能画：行号栏退化成标记列。
        let legacy = serde_json::json!({
            "kind": "diff",
            "deletions": ["a"],
            "insertions": ["b"],
        });
        assert_eq!(
            ToolView::from_details(Some(&legacy), ""),
            ToolView::Diff {
                deletions: vec!["a".into()],
                insertions: vec!["b".into()],
                at_line: None,
            }
        );
    }

    #[test]
    fn a_result_without_details_stays_plain_text() {
        // Three cases that must all degrade to text rather than invent a shape:
        // no payload (a tool with nothing structured to say), a payload of
        // another kind, and a diff payload that is empty.
        for details in [
            None,
            Some(serde_json::json!({"kind": "shell", "exit_code": 0})),
            Some(serde_json::json!({"kind": "diff", "deletions": [], "insertions": []})),
        ] {
            let view = ToolView::from_details(details.as_ref(), "some output");
            assert_eq!(
                view,
                ToolView::Plain {
                    text: "some output".into()
                },
                "不该凭空造出 diff: {details:?}"
            );
        }
    }

    #[test]
    fn duration_only_shows_up_when_waiting_was_the_point() {
        // Sub-second calls are noise ("5ms" on every read); a call worth waiting
        // for says how long it took.
        let fast = ToolOutcome {
            text: "x",
            ok: true,
            details: None,
            duration_ms: 240,
        };
        assert_eq!(fast.duration_label(), None);
        let slow = ToolOutcome {
            duration_ms: 12_400,
            ..fast
        };
        assert_eq!(slow.duration_label().as_deref(), Some("12.4s"));
        // …and it lands in the header meta of a finished card.
        let b = ToolBlock {
            name: "bash",
            args: r#"{"intent":"跑","command":"make"}"#,
            result: Some(slow),
            expanded: false,
        };
        let header = text_of(&[b.header(&t())]);
        assert!(header.contains("12.4s"), "header 缺耗时: {header}");
    }

    #[test]
    fn durations_read_the_way_omp_reads_them() {
        // Copied from omp's `formatDuration` — same thresholds, same units.
        assert_eq!(format_duration(0), "0ms");
        assert_eq!(format_duration(999), "999ms");
        assert_eq!(format_duration(1_000), "1.0s");
        assert_eq!(format_duration(59_900), "59.9s");
        assert_eq!(format_duration(60_000), "1m");
        assert_eq!(format_duration(150_000), "2m30s");
        assert_eq!(format_duration(3_600_000), "1h");
        assert_eq!(format_duration(3_660_000), "1h1m");
        assert_eq!(format_duration(90_000_000), "1d1h");
    }
}

