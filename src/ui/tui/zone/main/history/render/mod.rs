//! The painter: content in, rows out. One entry point.

pub mod block;
pub mod cards;
pub mod text;
pub mod theme;

use ratatui::style::Modifier;
use ratatui::text::Line;

use self::theme::{HistoryTheme, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    User,
    Reasoning,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Streaming {
    Final,
    Live,
}

pub fn render_block(
    kind: BlockKind,
    content: &str,
    streaming: Streaming,
    width: usize,
    t: &HistoryTheme,
    defer: bool,
) -> (Vec<Line<'static>>, Vec<block::Deferred>) {
    match kind {
        BlockKind::User => {
            debug_assert_eq!(
                streaming,
                Streaming::Final,
                "用户消息永远是成品，不存在流式"
            );
            (cards::user_card(content, t, width), Vec::new())
        }
        BlockKind::Assistant => markdown::render_markdown(content, t, defer, width),
        BlockKind::Reasoning => {
            let (rows, _) = markdown::render_markdown(content, t, true, width);
            (reasoning_fold(rows, t), Vec::new())
        }
    }
}

fn reasoning_fold(rows: Vec<Line<'static>>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for mut line in rows {
        for sp in &mut line.spans {
            sp.style = sp
                .style
                .fg(t.get(Token::SystemText))
                .add_modifier(Modifier::ITALIC);
        }
        out.push(line);
    }
    out
}
