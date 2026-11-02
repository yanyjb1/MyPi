//! The painter: one block in, rows out.

pub(in crate::ui::tui::zone::main::history) mod block;
pub(in crate::ui::tui::zone::main::history) mod cards;
pub(in crate::ui::tui::zone::main::history) mod notice;
pub(in crate::ui::tui::zone::main::history) mod text;

use ratatui::style::Modifier;
use ratatui::text::Line;

use self::text::markdown;
use super::local::Level;
use super::segment::{self, Kind};
use super::shape::{Block, Kind as BlockKind};
use crate::ui::tui::theme::{HistoryTheme, Token};

/// What the fold switches say, and what a card has to work with.
pub(in crate::ui::tui::zone::main::history) struct Opts<'a> {
    /// The thinking chain's own copy.
    pub thinking: bool,
    /// Tool exchanges' own copy.
    pub tool: bool,
    /// What a folded thinking chain shows.
    pub placeholder: &'a str,
    /// Where the tools ran, for shortening paths.
    pub cwd: &'a str,
    /// Rows the history zone has this frame; a running tool's tail window takes these.
    pub rows: usize,
}

/// One block's rows at this width.
pub(in crate::ui::tui::zone::main::history) fn block(
    block: &Block,
    paired: Option<&Block>,
    t: &HistoryTheme,
    o: &Opts<'_>,
    width: usize,
) -> Vec<Line<'static>> {
    match block.kind {
        BlockKind::User => cards::user_card(body(block), t, width),
        BlockKind::Wake => notice::notice_block(body(block), Level::Info, t),
        BlockKind::Assistant => paint(body(block), t, width),
        BlockKind::Reasoning => {
            if o.thinking {
                reasoning(paint(body(block), t, width), t)
            } else {
                vec![Line::from(
                    t.fg(o.placeholder.to_string(), Token::SystemText),
                )]
            }
        }
        BlockKind::ToolRequest => cards::chat::tool_node(block, paired, t, o, width),
        BlockKind::ToolResult => Vec::new(),
    }
}

/// A block's own text.
pub(in crate::ui::tui::zone::main::history) fn body(block: &Block) -> &str {
    block
        .content
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or("")
}

/// Prose: one segment at a time, each laid out on its own.
pub(in crate::ui::tui::zone::main::history) fn paint(
    text: &str,
    t: &HistoryTheme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for seg in segment::split(text) {
        match seg.kind {
            Kind::Code => {
                for line in text::highlight::highlight(&seg.source, seg.lang.as_deref(), t) {
                    out.extend(block::wrap_rows(std::iter::once(line), width));
                }
            }
            _ => out.extend(markdown::render_markdown(&seg.source, t, width)),
        }
    }
    out
}

/// The thinking chain's own look: muted and leaning.
fn reasoning(rows: Vec<Line<'static>>, t: &HistoryTheme) -> Vec<Line<'static>> {
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
