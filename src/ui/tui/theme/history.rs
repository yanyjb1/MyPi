//! The color snapshot for one frame: tokens in, colors out.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::ui::tui::theme::{ColorToken, theme};

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Token {
            $($name,)*
        }

        pub const ALL_TOKENS: &[Token] = &[$(Token::$name,)*];

        impl Token {
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

tokens! {
    UserCardBg,
    UserCardText,
    UserCardBar,

    AssistantText,
    ReasoningText,

    ToolTitle,
    ToolPath,
    ToolOutput,
    ToolBgPending,
    ToolBgSuccess,
    ToolBgError,
    ToolEdgePending,
    ToolEdgeSuccess,
    ToolEdgeError,

    Ok,
    Err,
    Warn,
    Dim,
    Muted,

    DiffAdded,
    DiffRemoved,
    DiffContext,

    MdHeading,
    MdLink,
    MdLinkUrl,
    MdCode,
    MdCodeBlockBorder,
    MdTableBorder,
    MdQuote,
    MdQuoteBorder,
    MdHr,
    MdListBullet,

    SyntaxComment,
    SyntaxKeyword,
    SyntaxFunction,
    SyntaxVariable,
    SyntaxString,
    SyntaxNumber,
    SyntaxType,
    SyntaxOperator,
    SyntaxPunctuation,

    SystemText,
    ErrorText,

    TreeUser,
    TreeAssistant,
    TreeToolCall,
    TreeToolResult,
    TreeReasoning,
    TreeLine,
    TreeAxis,
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryTheme {
    colors: [Color; ALL_TOKENS.len()],
}

impl HistoryTheme {
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
        put(
            Token::MdCodeBlockBorder,
            t.color(ColorToken::MdCodeBlockBorder),
        );
        put(Token::MdTableBorder, t.color(ColorToken::MdTableBorder));
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
        put(
            Token::SyntaxPunctuation,
            t.color(ColorToken::SyntaxPunctuation),
        );
        put(Token::TreeUser, t.color(ColorToken::Success));
        put(Token::TreeAssistant, t.color(ColorToken::Accent));
        put(Token::TreeToolCall, t.color(ColorToken::Warning));
        put(Token::TreeToolResult, t.color(ColorToken::SyntaxString));
        put(Token::TreeReasoning, t.color(ColorToken::ThinkingText));
        put(Token::TreeLine, t.color(ColorToken::Dim));
        put(Token::TreeAxis, t.color(ColorToken::Muted));
        put(Token::SystemText, t.color(ColorToken::Muted));
        put(Token::ErrorText, t.color(ColorToken::Error));
        Self { colors }
    }

    pub fn get(&self, tok: Token) -> Color {
        self.colors[tok.idx()]
    }

    pub fn fg(&self, text: impl Into<String>, tok: Token) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(tok)))
    }

    pub fn on(&self, text: impl Into<String>, fg: Token, bg: Token) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(fg)).bg(self.get(bg)))
    }

    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }

    pub fn fg_style(&self, tok: Token) -> Style {
        Style::new().fg(self.get(tok))
    }

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
