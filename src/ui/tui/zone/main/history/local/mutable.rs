//! The mutable region: the running turn's blocks, still growing.
//!
//! A block here has no upstream id yet; the round's receipt gives it one.

use super::content::{Block, Kind as BlockKind};

/// The blocks of one turn, in source order.
#[derive(Default)]
pub struct Mutable {
    blocks: Vec<Block>,
    open: Option<usize>,
}

impl Mutable {
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.open = None;
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn get(&self, at: usize) -> Option<&Block> {
        self.blocks.get(at)
    }

    pub fn take(&mut self) -> Vec<Block> {
        self.open = None;
        std::mem::take(&mut self.blocks)
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
    }

    /// A message this front end just put out: it leads the tail, above the reply.
    pub fn echo(&mut self, text: &str) {
        let mut block = blank(BlockKind::User, None);
        block.text = Some(text.to_string());
        self.blocks.insert(0, block);
        if let Some(open) = self.open.as_mut() {
            *open += 1;
        }
    }

    /// The echo opened nothing: the leading message goes.
    pub fn retract(&mut self) {
        if self
            .blocks
            .first()
            .is_some_and(|block| block.kind == BlockKind::User)
        {
            self.blocks.remove(0);
            if let Some(open) = self.open.as_mut() {
                *open = open.saturating_sub(1);
            }
        }
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

    /// Append to the open block of this kind, or open one.
    fn grow(&mut self, kind: BlockKind, piece: &str) {
        let at = match self.open.filter(|at| self.blocks[*at].kind == kind) {
            Some(at) => at,
            None => {
                let mut block = blank(kind, None);
                block.text = Some(String::new());
                self.blocks.push(block);
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
                self.blocks.len() - 1
            })
    }

    fn result(&mut self, call_id: &str) -> usize {
        self.find(BlockKind::ToolResult, call_id)
            .unwrap_or_else(|| {
                let mut block = blank(BlockKind::ToolResult, Some(call_id));
                block.result = Some(String::new());
                self.blocks.push(block);
                self.blocks.len() - 1
            })
    }

    fn find(&self, kind: BlockKind, call_id: &str) -> Option<usize> {
        self.blocks
            .iter()
            .position(|block| block.kind == kind && block.call_id.as_deref() == Some(call_id))
    }
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
