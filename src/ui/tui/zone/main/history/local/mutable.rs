//! The mutable region: the running turn's entries, still growing.
//!
//! Blocks here have no upstream id yet; the round's receipt gives them one. A notice is a line
//! only this front end says: it takes no id and never goes upstream.

use std::collections::HashMap;

use super::content::{Block, Kind as BlockKind, Truncation};
use super::slot::{Level, Life};
use crate::ui::tui::zone::main::history::product::Product;

/// One entry of the running turn, in source order.
pub enum Entry {
    /// A block the upstream will number.
    Block(Block),
    /// A line this front end has to say.
    Notice {
        text: String,
        level: Level,
        life: Life,
    },
}

/// Where one turn's entries grow: the blocks in source order, and the one still open.
#[derive(Default)]
struct Region {
    entries: Vec<Entry>,
    open: Option<usize>,
    /// The product each position handed over at the current width; a write clears it.
    products: HashMap<usize, Product>,
}

impl Region {
    fn touch(&mut self) {
        self.products.clear();
    }
}

/// The running turn. While a close waits for its rows, the next turn's entries grow beside it.
#[derive(Default)]
pub struct Mutable {
    live: Region,
    next: Option<Region>,
}

impl Mutable {
    pub fn clear(&mut self) {
        self.live = Region::default();
        self.next = None;
    }

    /// A close is waiting for its rows: the next turn's entries stay out of the way.
    pub fn seal(&mut self) {
        self.next = Some(Region::default());
    }

    /// The close settled: the turn goes, and whatever the next one already said takes its place.
    pub fn adopt(&mut self) {
        self.live = self.next.take().unwrap_or_default();
    }

    /// The product this position handed over, when it was built.
    pub fn product(&self, at: usize) -> Option<&Product> {
        self.live.products.get(&at)
    }

    pub fn set_product(&mut self, at: usize, product: Product) {
        self.live.products.insert(at, product);
    }

    pub fn drop_product(&mut self, at: usize) {
        self.live.products.remove(&at);
    }

    /// Every built product goes: the options or the width moved.
    pub fn drop_products(&mut self) {
        self.live.touch();
        if let Some(next) = self.next.as_mut() {
            next.touch();
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.live.entries
    }

    pub fn len(&self) -> usize {
        self.live.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.entries.is_empty()
    }

    pub fn get(&self, at: usize) -> Option<&Entry> {
        self.live.entries.get(at)
    }

    pub fn block(&self, at: usize) -> Option<&Block> {
        match self.live.entries.get(at) {
            Some(Entry::Block(block)) => Some(block),
            _ => None,
        }
    }

    /// The result block of this call, when it is already in hand.
    pub fn result_of(&self, call_id: &str) -> Option<&Block> {
        let at = self.live.find(BlockKind::ToolResult, call_id)?;
        self.block(at)
    }

    /// The position of this call's request, when it is already in hand.
    pub fn request_at(&self, call_id: &str) -> Option<usize> {
        self.live.find(BlockKind::ToolRequest, call_id)
    }

    pub fn take(&mut self) -> Vec<Entry> {
        self.live.open = None;
        self.live.touch();
        std::mem::take(&mut self.live.entries)
    }

    pub fn text(&mut self, piece: &str) -> usize {
        let region = self.target();
        region.touch();
        region.grow(BlockKind::Assistant, piece)
    }

    pub fn thinking(&mut self, piece: &str) -> usize {
        let region = self.target();
        region.touch();
        region.grow(BlockKind::Reasoning, piece)
    }

    pub fn boundary(&mut self) {
        self.target().open = None;
    }

    /// One wake message, its own block: the text names the reminder.
    pub fn wake(&mut self, text: &str) -> usize {
        let region = self.target();
        region.touch();
        let mut block = blank(BlockKind::Wake, None);
        block.text = Some(text.to_string());
        region.open = None;
        region.entries.push(Entry::Block(block));
        region.entries.len() - 1
    }

    /// A message this front end just put out: it leads the turn, above the reply.
    pub fn echo(&mut self, text: &str) -> usize {
        let region = self.target();
        region.touch();
        let mut block = blank(BlockKind::User, None);
        block.text = Some(text.to_string());
        region.entries.insert(0, Entry::Block(block));
        if let Some(open) = region.open.as_mut() {
            *open += 1;
        }
        0
    }

    /// The echo opened nothing: the leading message goes.
    pub fn retract(&mut self) {
        let region = self.target();
        region.touch();
        let leading = matches!(region.entries.first(), Some(Entry::Block(block)) if block.kind == BlockKind::User);
        if leading {
            region.entries.remove(0);
            if let Some(open) = region.open.as_mut() {
                *open = open.saturating_sub(1);
            }
        }
    }

    /// A line only this front end says: it closes whatever block was still growing.
    pub fn notice(&mut self, text: String, level: Level, life: Life) -> usize {
        let region = self.target();
        region.touch();
        region.open = None;
        region.entries.push(Entry::Notice { text, level, life });
        region.entries.len() - 1
    }

    pub fn arguments(&mut self, call_id: &str, name: &str, piece: &str) -> usize {
        let region = self.target();
        region.touch();
        let at = region.request(call_id);
        let Some(Entry::Block(block)) = region.entries.get_mut(at) else {
            return at;
        };
        if !name.is_empty() && block.name.as_deref().is_none_or(str::is_empty) {
            block.name = Some(name.to_string());
        }
        block.args.get_or_insert_with(String::new).push_str(piece);
        at
    }

    pub fn tool_start(
        &mut self,
        call_id: &str,
        name: &str,
        args: &str,
        intent: Option<&str>,
    ) -> usize {
        let region = self.target();
        region.touch();
        let at = region.request(call_id);
        let Some(Entry::Block(block)) = region.entries.get_mut(at) else {
            return at;
        };
        block.name = Some(name.to_string());
        block.args = Some(args.to_string());
        block.intent = intent.map(str::to_string);
        at
    }

    pub fn tool_progress(&mut self, call_id: &str, piece: &str) -> usize {
        let region = self.target();
        region.touch();
        let at = region.result(call_id);
        let Some(Entry::Block(block)) = region.entries.get_mut(at) else {
            return at;
        };
        block.result.get_or_insert_with(String::new).push_str(piece);
        at
    }

    pub fn tool_end(
        &mut self,
        call_id: &str,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        duration_ms: i64,
        truncated: Option<Truncation>,
    ) -> usize {
        let region = self.target();
        region.touch();
        let at = region.result(call_id);
        let Some(Entry::Block(block)) = region.entries.get_mut(at) else {
            return at;
        };
        block.ok = Some(ok);
        block.result = Some(result.to_string());
        block.memo = memo.map(str::to_string);
        block.duration_ms = duration_ms;
        block.truncated = truncated;
        at
    }

    /// Where this turn's writes land: the live region, or the one a seal opened.
    fn target(&mut self) -> &mut Region {
        self.next.as_mut().unwrap_or(&mut self.live)
    }
}

impl Region {
    /// Append to the open block of this kind, or open one.
    fn grow(&mut self, kind: BlockKind, piece: &str) -> usize {
        let at = match self.open.filter(|at| {
            matches!(
                self.entries.get(*at),
                Some(Entry::Block(block)) if block.kind == kind
            )
        }) {
            Some(at) => at,
            _ => {
                let mut block = blank(kind, None);
                block.text = Some(String::new());
                self.entries.push(Entry::Block(block));
                let at = self.entries.len() - 1;
                self.open = Some(at);
                at
            }
        };
        let Some(Entry::Block(block)) = self.entries.get_mut(at) else {
            return at;
        };
        if let Some(text) = block.text.as_mut() {
            text.push_str(piece);
        }
        at
    }

    fn find(&self, kind: BlockKind, call_id: &str) -> Option<usize> {
        self.entries.iter().position(|entry| match entry {
            Entry::Block(block) => block.kind == kind && block.call_id.as_deref() == Some(call_id),
            Entry::Notice { .. } => false,
        })
    }

    fn request(&mut self, call_id: &str) -> usize {
        self.find(BlockKind::ToolRequest, call_id)
            .unwrap_or_else(|| {
                let mut block = blank(BlockKind::ToolRequest, Some(call_id));
                block.ok = None;
                self.entries.push(Entry::Block(block));
                self.entries.len() - 1
            })
    }

    fn result(&mut self, call_id: &str) -> usize {
        self.find(BlockKind::ToolResult, call_id)
            .unwrap_or_else(|| {
                let mut block = blank(BlockKind::ToolResult, Some(call_id));
                block.result = Some(String::new());
                self.entries.push(Entry::Block(block));
                self.entries.len() - 1
            })
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
        truncated: None,
    }
}
