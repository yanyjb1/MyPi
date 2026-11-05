//! The tail that is still growing: upstream's live pieces, gathered into blocks.

use super::super::layout::line::{Lead, Row};
use super::super::layout::segment::{self, Kind};
use super::super::paint::{self, Opts};
use super::decode::{Block, Kind as BlockKind};

/// One block's finished segments: source length in, natural rows out.
#[derive(Default)]
struct Segments {
    segments: Vec<(usize, Vec<Row>)>,
    /// The open fence, fed one line at a time.
    fence: Option<Fence>,
}

struct Fence {
    painter: paint::code::Painter,
    fed: usize,
    rows: Vec<Row>,
}

#[derive(Default)]
pub struct Live {
    blocks: Vec<Block>,
    open: Option<usize>,
    laid: Vec<Segments>,
}

impl Live {
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.laid.clear();
        self.open = None;
    }

    pub fn text(&mut self, piece: &str) {
        self.grow(BlockKind::Assistant, piece);
    }

    pub fn thinking(&mut self, piece: &str) {
        self.grow(BlockKind::Reasoning, piece);
    }

    pub fn boundary(&mut self) {
        self.open = None;
    }

    /// One wake message, its own block: the text names the reminder.
    pub fn wake(&mut self, text: &str) {
        let mut block = blank(BlockKind::Wake, None);
        block.text = Some(text.to_string());
        self.open = None;
        self.blocks.push(block);
        self.laid.push(Segments::default());
    }

    /// A message this front end just put out: it leads the tail, above the reply.
    pub fn echo(&mut self, text: &str) {
        let mut block = blank(BlockKind::User, None);
        block.text = Some(text.to_string());
        self.blocks.insert(0, block);
        self.laid.insert(0, Segments::default());
        if let Some(open) = self.open.as_mut() {
            *open += 1;
        }
    }

    /// The echo opened nothing: the leading message goes.
    pub fn retract(&mut self) {
        if self
            .blocks
            .first()
            .is_some_and(|b| b.kind == BlockKind::User)
        {
            self.blocks.remove(0);
            self.laid.remove(0);
            if let Some(open) = self.open.as_mut() {
                *open = open.saturating_sub(1);
            }
        }
    }

    /// How many blocks the tail holds.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Nothing is growing.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The turn is over: the blocks leave, and the rows with them.
    pub fn take(&mut self) -> Vec<Block> {
        self.open = None;
        self.laid.clear();
        std::mem::take(&mut self.blocks)
    }

    pub fn arguments(&mut self, call_id: &str, piece: &str) {
        let at = self.request(call_id);
        let block = &mut self.blocks[at];
        block.args.get_or_insert_with(String::new).push_str(piece);
    }

    pub fn tool_start(&mut self, call_id: &str, name: &str, args: &str, intent: Option<&str>) {
        let at = self.request(call_id);
        let block = &mut self.blocks[at];
        block.name = Some(name.to_string());
        block.args = Some(args.to_string());
        block.intent = intent.map(str::to_string);
    }

    pub fn tool_progress(&mut self, call_id: &str, piece: &str) {
        let at = self.result(call_id);
        let block = &mut self.blocks[at];
        block.result.get_or_insert_with(String::new).push_str(piece);
    }

    pub fn tool_end(
        &mut self,
        call_id: &str,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        duration_ms: i64,
    ) {
        let at = self.result(call_id);
        let block = &mut self.blocks[at];
        block.ok = Some(ok);
        block.result = Some(result.to_string());
        block.memo = memo.map(str::to_string);
        block.duration_ms = duration_ms;
    }

    /// Every block of the running turn, as rows at this width.
    pub fn rows(&mut self, opts: &Opts<'_>, width: usize) -> Vec<Row> {
        let mut out: Vec<Row> = Vec::new();
        for at in 0..self.blocks.len() {
            if self.blocks[at].kind == BlockKind::ToolResult {
                continue;
            }
            let paired = self
                .blocks
                .get(at + 1)
                .cloned()
                .filter(|next| self.blocks[at].answers(next));
            let rows = self.block_rows(at, paired.as_ref(), opts, width);
            if rows.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(Row::blank());
            }
            out.extend(rows);
        }
        out
    }

    fn block_rows(
        &mut self,
        at: usize,
        paired: Option<&Block>,
        opts: &Opts<'_>,
        width: usize,
    ) -> Vec<Row> {
        match self.blocks[at].kind {
            BlockKind::ToolRequest => paint::card::node(&self.blocks[at], paired, opts, width),
            BlockKind::User => paint::user::block(&self.blocks[at], width),
            BlockKind::Wake => paint::notice::block(
                self.blocks[at].text.as_deref().unwrap_or_default(),
                super::super::local::Level::Info,
            ),
            BlockKind::Assistant | BlockKind::Reasoning => {
                let rows = self.text_rows(at, width);
                if self.blocks[at].kind == BlockKind::Reasoning {
                    if opts.thinking {
                        paint::prose::thinking(rows)
                    } else {
                        vec![paint::prose::placeholder(opts.placeholder)]
                    }
                } else {
                    rows
                }
            }
            BlockKind::ToolResult => Vec::new(),
        }
    }

    fn text_rows(&mut self, at: usize, width: usize) -> Vec<Row> {
        let text = self.blocks[at]
            .content
            .as_deref()
            .or(self.blocks[at].text.as_deref())
            .unwrap_or_default()
            .to_string();
        let segments = segment::split(&text);
        let last = segments.len().saturating_sub(1);
        let mut out: Vec<Row> = Vec::new();
        for (index, seg) in segments.iter().enumerate() {
            let open = index == last && !closed(seg);
            if !open {
                let rows = self.closed_rows(at, index, seg, width);
                out.extend(super::super::layout::wrap::rows(rows, width));
                continue;
            }
            if seg.kind == Kind::Code {
                let rows = self.fence_rows(at, seg);
                out.extend(super::super::layout::wrap::rows(rows, width));
            } else {
                let rows = match seg.kind {
                    Kind::Table => paint::table::rows(seg, width),
                    _ => paint::prose::text(seg),
                };
                out.extend(super::super::layout::wrap::rows(rows, width));
            }
        }
        out
    }

    fn closed_rows(
        &mut self,
        at: usize,
        index: usize,
        seg: &segment::Segment,
        width: usize,
    ) -> Vec<Row> {
        let state = &mut self.laid[at];
        if let Some((len, rows)) = state.segments.get(index)
            && *len == seg.source.len()
        {
            return rows.clone();
        }
        let rows = match seg.kind {
            Kind::Code => paint::code::rows(seg),
            Kind::Table => paint::table::rows(seg, width),
            _ => paint::prose::text(seg),
        };
        let entry = (seg.source.len(), rows.clone());
        if state.segments.len() > index {
            state.segments[index] = entry;
        } else {
            state.segments.push(entry);
        }
        rows
    }

    fn fence_rows(&mut self, at: usize, seg: &segment::Segment) -> Vec<Row> {
        let opener = seg
            .source
            .lines()
            .next()
            .unwrap_or("```")
            .trim_end()
            .to_string();
        let state = &mut self.laid[at];
        let fence = state.fence.get_or_insert_with(|| Fence {
            painter: paint::code::Painter::new(seg.lang.as_deref()),
            fed: 0,
            rows: vec![Row::of(
                Lead::First,
                vec![super::super::layout::line::Run::fixed(
                    opener,
                    crate::ui::tui::theme::fixed::MD_CODE_BLOCK_BORDER,
                )],
            )],
        });
        let source = &seg.source;
        let mut at_byte = fence.fed;
        while at_byte < source.len() {
            let Some(rel) = source[at_byte..].find('\n') else {
                break;
            };
            let end = at_byte + rel + 1;
            let line = &source[at_byte..end];
            if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                break;
            }
            fence.rows.push(fence.painter.push(line));
            at_byte = end;
        }
        fence.fed = at_byte;
        fence.rows.clone()
    }

    /// Append to the open block of this kind, or open one.
    fn grow(&mut self, kind: BlockKind, piece: &str) {
        let at = match self.open.filter(|at| self.blocks[*at].kind == kind) {
            Some(at) => at,
            None => {
                let mut block = blank(kind, None);
                block.text = Some(String::new());
                self.blocks.push(block);
                self.laid.push(Segments::default());
                let at = self.blocks.len() - 1;
                self.open = Some(at);
                at
            }
        };
        if let Some(text) = self.blocks[at].text.as_mut() {
            text.push_str(piece);
        }
    }

    fn request(&mut self, call_id: &str) -> usize {
        self.find(BlockKind::ToolRequest, call_id)
            .unwrap_or_else(|| {
                let mut block = blank(BlockKind::ToolRequest, Some(call_id));
                block.ok = None;
                self.blocks.push(block);
                self.laid.push(Segments::default());
                self.blocks.len() - 1
            })
    }

    fn result(&mut self, call_id: &str) -> usize {
        self.find(BlockKind::ToolResult, call_id)
            .unwrap_or_else(|| {
                let mut block = blank(BlockKind::ToolResult, Some(call_id));
                block.result = Some(String::new());
                self.blocks.push(block);
                self.laid.push(Segments::default());
                self.blocks.len() - 1
            })
    }

    fn find(&self, kind: BlockKind, call_id: &str) -> Option<usize> {
        self.blocks
            .iter()
            .position(|block| block.kind == kind && block.call_id.as_deref() == Some(call_id))
    }
}

/// A segment stops growing only when a fence closes it or another segment follows.
fn closed(seg: &segment::Segment) -> bool {
    if seg.kind != Kind::Code {
        return false;
    }
    let last = seg.source.lines().next_back().unwrap_or_default();
    let trimmed = last.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn blank(kind: BlockKind, call_id: Option<&str>) -> Block {
    Block {
        id: 0,
        kind,
        content: None,
        text: None,
        name: None,
        args: None,
        intent: None,
        call_id: call_id.map(str::to_string),
        first: false,
        ok: None,
        result: None,
        memo: None,
        duration_ms: 0,
    }
}
