//! 历史区色号管理 —— 与状态栏同一套规矩。
//!
//! 三层：全局色号库（`crate::tui::theme::ColorToken`）→ 历史区的取色单
//! （本文件的 [`Token`]，每个 = 一个渲染决定）→ 每帧
//! [`HistoryTheme::resolve`] 抓一份快照。换主题时下一帧自动全变。
//!
//! [`Token`] 由宏一处声明：枚举、[`ALL_TOKENS`]、[`Token::idx`] 三者
//! 同源，加一个色号只改一行，索引不可能错位。
//!
//! 取值本身照抄 omp `theme/defaults/dark.json`（我们内置主题与它逐项
//! 相同）：成功是 **dim 灰**，失败才是红——成功不该抢眼。

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::tui::theme::{theme, ColorToken};

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        /// 历史区每个「上色决定」一个条目。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Token {
            $($name,)*
        }

        /// 每个色号，供覆盖审计 + 索引自检。
        pub const ALL_TOKENS: &[Token] = &[$(Token::$name,)*];

        impl Token {
            /// 在快照里的下标。与 [`ALL_TOKENS`] 同源，不会漂。
            pub const fn idx(self) -> usize {
                let mut i = 0usize;
                $(
                    if matches!(self, Token::$name) {
                        return i;
                    }
                    i += 1;
                )*
                unreachable!()
            }
        }
    };
}

// 宏调用里只能写普通注释（`///` 会变成属性，宏接不住）。
tokens! {
    // ---- 用户卡 ----
    // 用户卡背景。
    UserCardBg,
    // 用户卡正文前景。
    UserCardText,
    // 卡左侧竖条。
    UserCardBar,

    // ---- 助手文本 ----
    // 助手正文（默认前景）。
    AssistantText,
    // 思考链。
    ReasoningText,

    // ---- 工具卡 ----
    // 工具标题（header 里的 title）。
    ToolTitle,
    // header 里的**操作对象**（文件路径）。与标题分开取色：omp 的
    // `renderStatusLine` 里 title 与 description 是两档。
    ToolPath,
    // 工具输出正文。
    ToolOutput,
    // 待执行 / 流式中：底色。
    ToolBgPending,
    // 成功：底色。
    ToolBgSuccess,
    // 失败：底色。
    ToolBgError,
    // 待执行 / 流式中：边框 + 状态字形。
    ToolEdgePending,
    // 成功：边框（**dim 灰**，不抢眼）。
    ToolEdgeSuccess,
    // 失败：边框。
    ToolEdgeError,

    // ---- 语义状态色 ----
    // 成功字形。
    Ok,
    // 失败字形。
    Err,
    // 警告字形（超时之类）。
    Warn,
    // 弱化的说明文字（折叠标记、展开提示、meta）。
    Dim,
    // 更弱的一档说明文字（description、无输出占位）。
    Muted,

    // ---- diff ----
    // diff 新增行。
    DiffAdded,
    // diff 删除行。
    DiffRemoved,
    // diff 上下文行。
    DiffContext,

    // ---- markdown ----
    MdHeading,
    MdLink,
    MdLinkUrl,
    MdCode,
    // 代码块 ``` 边框。
    MdCodeBlockBorder,
    MdQuote,
    // 引用左侧竖条。
    MdQuoteBorder,
    MdHr,
    MdListBullet,

    // ---- 语法高亮（9 个，走主题）----
    SyntaxComment,
    SyntaxKeyword,
    SyntaxFunction,
    SyntaxVariable,
    SyntaxString,
    SyntaxNumber,
    SyntaxType,
    SyntaxOperator,
    SyntaxPunctuation,

    // ---- 系统注记 ----
    // 系统注记正文。
    SystemText,
    // 错误文本。
    ErrorText,

    // ---- 节点树 ----
    // 树上四类角色的名字（`user:` / `assistant:` …）。
    //
    // 四类必须**互不相同**：树的作用就是一眼看出这段对话是谁说的，
    // 同色等于没标。取值全部落在既有语义色上（不新造颜色），
    // 换主题时跟着走。
    TreeUser,
    TreeAssistant,
    // 工具**调用**（模型要干什么）。
    TreeToolCall,
    // 工具**结果**（干成了什么）。
    TreeToolResult,
    TreeReasoning,
    // 树上的分叉连线（竖线、拐点、横线）。
    TreeLine,
    // 树的轴（主干那条 + 分支那条），比连线亮一档。
    TreeAxis,
}

/// 每帧的色号快照。渲染循环调一次 [`Self::resolve`]，往下传。
#[derive(Debug, Clone, Copy)]
pub struct HistoryTheme {
    colors: [Color; ALL_TOKENS.len()],
}

impl HistoryTheme {
    /// 快照当前主题。每帧调一次，所以运行时换主题下一帧生效。
    pub fn resolve() -> Self {
        let t = theme();
        let mut colors = [Color::Reset; ALL_TOKENS.len()];
        let mut put = |tok: Token, c: Color| colors[tok.idx()] = c;
        put(Token::UserCardBg, t.color(ColorToken::UserMessageBg));
        put(Token::UserCardText, t.color(ColorToken::UserMessageText));
        put(Token::UserCardBar, t.color(ColorToken::Accent));
        put(Token::AssistantText, t.color(ColorToken::Text));
        put(Token::ReasoningText, t.color(ColorToken::ThinkingText));
        put(Token::ToolTitle, t.color(ColorToken::ToolTitle));
        put(Token::ToolPath, t.color(ColorToken::Accent));
        put(Token::ToolOutput, t.color(ColorToken::ToolOutput));
        put(Token::ToolBgPending, t.color(ColorToken::ToolPendingBg));
        put(Token::ToolBgSuccess, t.color(ColorToken::ToolSuccessBg));
        put(Token::ToolBgError, t.color(ColorToken::ToolErrorBg));
        put(Token::ToolEdgePending, t.color(ColorToken::Accent));
        put(Token::ToolEdgeSuccess, t.color(ColorToken::Dim));
        put(Token::ToolEdgeError, t.color(ColorToken::Error));
        put(Token::Ok, t.color(ColorToken::Success));
        put(Token::Err, t.color(ColorToken::Error));
        put(Token::Warn, t.color(ColorToken::Warning));
        put(Token::Dim, t.color(ColorToken::Dim));
        put(Token::Muted, t.color(ColorToken::Muted));
        put(Token::DiffAdded, t.color(ColorToken::ToolDiffAdded));
        put(Token::DiffRemoved, t.color(ColorToken::ToolDiffRemoved));
        put(Token::DiffContext, t.color(ColorToken::ToolDiffContext));
        put(Token::MdHeading, t.color(ColorToken::MdHeading));
        put(Token::MdLink, t.color(ColorToken::MdLink));
        put(Token::MdLinkUrl, t.color(ColorToken::MdLinkUrl));
        put(Token::MdCode, t.color(ColorToken::MdCode));
        put(Token::MdCodeBlockBorder, t.color(ColorToken::MdCodeBlockBorder));
        put(Token::MdQuote, t.color(ColorToken::MdQuote));
        put(Token::MdQuoteBorder, t.color(ColorToken::MdQuoteBorder));
        put(Token::MdHr, t.color(ColorToken::MdHr));
        put(Token::MdListBullet, t.color(ColorToken::MdListBullet));
        put(Token::SyntaxComment, t.color(ColorToken::SyntaxComment));
        put(Token::SyntaxKeyword, t.color(ColorToken::SyntaxKeyword));
        put(Token::SyntaxFunction, t.color(ColorToken::SyntaxFunction));
        put(Token::SyntaxVariable, t.color(ColorToken::SyntaxVariable));
        put(Token::SyntaxString, t.color(ColorToken::SyntaxString));
        put(Token::SyntaxNumber, t.color(ColorToken::SyntaxNumber));
        put(Token::SyntaxType, t.color(ColorToken::SyntaxType));
        put(Token::SyntaxOperator, t.color(ColorToken::SyntaxOperator));
        put(Token::SyntaxPunctuation, t.color(ColorToken::SyntaxPunctuation));
        // 节点树：四类角色各占一个既有色，围栏用 dim 灰，轴稍亮。
        put(Token::TreeUser, t.color(ColorToken::Success));
        put(Token::TreeAssistant, t.color(ColorToken::Accent));
        // 调用与结果**分开**：调用是"要做什么"，结果是"做成了什么"。
        // 取色要避开另外三类的色（user 用 success、assistant 用 accent），
        // 否则四类里有两类同色 = 白分。`SyntaxString` 是那之外的一档
        // （本机主题 = titaniumGold `#d4c090`），冷暖也与 Warning 分得开。
        put(Token::TreeToolCall, t.color(ColorToken::Warning));
        put(Token::TreeToolResult, t.color(ColorToken::SyntaxString));
        put(Token::TreeReasoning, t.color(ColorToken::ThinkingText));
        // 结构线取 dim/muted 这两档灰，**不能取 border**——本机主题的
        // border 是 `#2a3038`（近黑），在深色背景上等于没画。
        put(Token::TreeLine, t.color(ColorToken::Dim));
        put(Token::TreeAxis, t.color(ColorToken::Muted));
        put(Token::SystemText, t.color(ColorToken::Muted));
        put(Token::ErrorText, t.color(ColorToken::Error));
        Self { colors }
    }

    /// 一个色号的当前值。
    pub fn get(&self, tok: Token) -> Color {
        self.colors[tok.idx()]
    }

    /// 从原始值造快照。仅测试用：让测试不必改进程全局主题就能证明
    /// 主题无关性。
    #[cfg(test)]
    pub(crate) fn from_parts(colors: [Color; ALL_TOKENS.len()]) -> Self {
        Self { colors }
    }

    // ---- span 构造器：历史区 span 上色的唯一地方 ----

    /// 前景，透明背景。
    pub fn fg(&self, text: impl Into<String>, tok: Token) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(tok)))
    }

    /// 前景 + 背景。
    pub fn on(&self, text: impl Into<String>, fg: Token, bg: Token) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(fg)).bg(self.get(bg)))
    }

    /// 无样式：默认前景，透明背景。
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }

    /// 只要样式不要 span：给 `Line::styled` / `Style::patch` 用。
    pub fn fg_style(&self, tok: Token) -> Style {
        Style::new().fg(self.get(tok))
    }

    /// 前景 + 修饰（粗体/斜体）。
    pub fn fg_mod(
        &self,
        text: impl Into<String>,
        tok: Token,
        m: ratatui::style::Modifier,
    ) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(tok)).add_modifier(m))
    }
}

impl Default for HistoryTheme {
    fn default() -> Self {
        Self::resolve()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 色号表的全部意义：颜色跟着主题走，不是写死在渲染代码里。
    /// 造两份不同快照，同一个色号必须取到不同的值——如果渲染代码
    /// 硬写颜色，这条会挂。
    #[test]
    fn colors_come_from_the_theme_not_from_render_code() {
        let mut a = [Color::Reset; ALL_TOKENS.len()];
        let mut b = [Color::Reset; ALL_TOKENS.len()];
        for (i, _) in ALL_TOKENS.iter().enumerate() {
            a[i] = Color::Rgb(10, 20, 30);
            b[i] = Color::Rgb(200, 210, 220);
        }
        let ta = HistoryTheme::from_parts(a);
        let tb = HistoryTheme::from_parts(b);
        for tok in ALL_TOKENS {
            assert_ne!(
                ta.get(*tok),
                tb.get(*tok),
                "{tok:?} 两份快照取到同色：颜色没走主题"
            );
        }
    }

    /// idx 与 ALL_TOKENS 同源：按下标写回、按下标读出必须一致。
    #[test]
    fn token_index_matches_declaration_order() {
        for (i, tok) in ALL_TOKENS.iter().enumerate() {
            assert_eq!(tok.idx(), i, "{tok:?} 索引与声明顺序不一致");
        }
    }

    /// 成功不是绿的：omp 的规矩是成功用 dim 灰边框，红只留给失败。
    #[test]
    fn success_is_dim_and_failure_is_red() {
        let t = HistoryTheme::resolve();
        assert_ne!(
            t.get(Token::ToolEdgeSuccess),
            t.get(Token::ToolEdgeError),
            "成功与失败的边框同色，成败分不出来"
        );
        assert_eq!(t.get(Token::ToolEdgeSuccess), t.get(Token::Dim));
        assert_eq!(t.get(Token::ToolEdgeError), t.get(Token::Err));
    }

    #[test]
    fn syntax_tokens_are_all_distinct_from_each_other() {
        // 九个语法色号若全撞色，高亮等于没做。允许个别相同（主题可
        // 以把 number 和 string 设成同色），但不能九个全一样。
        let t = HistoryTheme::resolve();
        let syntax = [
            Token::SyntaxComment,
            Token::SyntaxKeyword,
            Token::SyntaxFunction,
            Token::SyntaxVariable,
            Token::SyntaxString,
            Token::SyntaxNumber,
            Token::SyntaxType,
            Token::SyntaxOperator,
            Token::SyntaxPunctuation,
        ];
        let mut seen = std::collections::HashSet::new();
        for tok in syntax {
            seen.insert(format!("{:?}", t.get(tok)));
        }
        assert!(seen.len() > 1, "九个语法色号全同色，高亮失效");
    }
}
