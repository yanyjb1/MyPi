//! The tail that is still growing: upstream's live pieces, gathered into blocks.

use super::shape::{Block, Kind, Usage};

/// The blocks a running turn has produced so far. They carry no block id: a number
/// is given the moment the round closes its ranges.
#[derive(Debug, Default)]
pub struct Live {
    blocks: Vec<Block>,
    /// The block still taking pieces, if any.
    open: Option<usize>,
}

impl Live {
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.open = None;
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// One piece of the reply's own text.
    pub fn text(&mut self, piece: &str) {
        self.grow(Kind::Assistant, piece);
    }

    /// One piece of the thinking chain.
    pub fn thinking(&mut self, piece: &str) {
        self.grow(Kind::Reasoning, piece);
    }

    /// The block ends here: the next piece opens a new one.
    pub fn boundary(&mut self) {
        self.open = None;
    }

    /// One fragment of a tool call's arguments.
    pub fn arguments(&mut self, call_id: &str, piece: &str) {
        let at = self.request(call_id);
        let args = self.blocks[at].args.get_or_insert_with(String::new);
        args.push_str(piece);
    }

    /// A tool call the model asked for.
    pub fn tool_start(&mut self, call_id: &str, name: &str, args: &str, intent: Option<&str>) {
        let at = self.request(call_id);
        if !name.is_empty() {
            self.blocks[at].name = Some(name.to_string());
        }
        if !args.is_empty() {
            self.blocks[at].args = Some(args.to_string());
        }
        if let Some(intent) = intent {
            self.blocks[at].intent = Some(intent.to_string());
        }
    }

    /// What a running tool printed.
    pub fn tool_progress(&mut self, call_id: &str, piece: &str) {
        let at = self.result(call_id);
        let result = self.blocks[at].result.get_or_insert_with(String::new);
        result.push_str(piece);
    }

    /// A tool call ended.
    pub fn tool_end(
        &mut self,
        call_id: &str,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        duration_ms: i64,
    ) {
        let at = self.result(call_id);
        self.blocks[at].ok = Some(ok);
        if !result.is_empty() {
            self.blocks[at].result = Some(result.to_string());
        }
        if let Some(memo) = memo {
            self.blocks[at].memo = Some(memo.to_string());
        }
        self.blocks[at].duration_ms = duration_ms;
    }

    /// Append to the open block of this kind, or open one.
    fn grow(&mut self, kind: Kind, piece: &str) {
        let at = match self.open {
            Some(at) if self.blocks[at].kind == kind => at,
            _ => {
                self.blocks.push(blank(kind, None));
                let at = self.blocks.len() - 1;
                self.open = Some(at);
                at
            }
        };
        let body = self.blocks[at].content.get_or_insert_with(String::new);
        body.push_str(piece);
    }

    /// The tool request with this call id, opened if it is not there.
    fn request(&mut self, call_id: &str) -> usize {
        self.find(Kind::ToolRequest, call_id).unwrap_or_else(|| {
            self.blocks.push(blank(Kind::ToolRequest, Some(call_id)));
            self.blocks.len() - 1
        })
    }

    /// The tool result with this call id, opened if it is not there.
    fn result(&mut self, call_id: &str) -> usize {
        self.find(Kind::ToolResult, call_id).unwrap_or_else(|| {
            self.blocks.push(blank(Kind::ToolResult, Some(call_id)));
            self.blocks.len() - 1
        })
    }

    fn find(&self, kind: Kind, call_id: &str) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.kind == kind && b.call_id.as_deref() == Some(call_id))
    }
}

/// One empty block of this kind and call id.
fn blank(kind: Kind, call_id: Option<&str>) -> Block {
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
        usage: Usage::default(),
    }
}
