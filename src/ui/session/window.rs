//! The raw cache: the landed window, ascending by id, plus the tail that has not landed.
//! Design: `README.md`.

use std::collections::VecDeque;

use crate::server::api::call::viewport::PlacedBlock;
use crate::server::api::types::TurnBlock;
use crate::server::storage::{Block, NewBlock};

/// Blocks kept beyond the budget, so one scroll does not ask again.
const SCREEN_BLOCKS: usize = 24;

/// What the view is missing: the session turns it into a read. `turn` = the round the view sits
/// on, so the answer can leave that round implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Older {
        before: i64,
        limit: usize,
        turn: Option<i64>,
    },
    Newer {
        after: i64,
        limit: usize,
        turn: Option<i64>,
    },
}

/// One block as a render side reads it: the one shape over `Block` and `NewBlock`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlockRef<'a> {
    Landed(&'a Block),
    Live(&'a NewBlock),
}

impl<'a> BlockRef<'a> {
    pub fn id(&self) -> Option<i64> {
        match self {
            BlockRef::Landed(b) => Some(b.id),
            BlockRef::Live(_) => None,
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self, BlockRef::Live(_))
    }

    pub fn kind(&self) -> crate::server::storage::BlockKind {
        match self {
            BlockRef::Landed(b) => b.kind,
            BlockRef::Live(b) => b.kind,
        }
    }

    /// The id-claim hash: the same recipe the server hashes with.
    pub fn content_key(&self) -> crate::server::storage::ContentKey<'a> {
        match self {
            BlockRef::Landed(b) => b.content_key(),
            BlockRef::Live(b) => b.content_key(),
        }
    }
}

macro_rules! borrowed_fields {
    ($($name:ident),* $(,)?) => {
        impl<'a> BlockRef<'a> {
            $(
                pub fn $name(&self) -> Option<&'a String> {
                    match self {
                        BlockRef::Landed(b) => b.$name.as_ref(),
                        BlockRef::Live(b) => b.$name.as_ref(),
                    }
                }
            )*
        }
    };
}

macro_rules! optional_fields {
    ($($name:ident : $ty:ty),* $(,)?) => {
        impl<'a> BlockRef<'a> {
            $(
                pub fn $name(&self) -> Option<$ty> {
                    match self {
                        BlockRef::Landed(b) => b.$name,
                        BlockRef::Live(b) => b.$name,
                    }
                }
            )*
        }
    };
}

borrowed_fields!(
    content, prefix, suffix, call_id, name, args, intent, text, result, memo, ts,
);

optional_fields!(
    first: bool,
    ok: bool,
    duration_ms: i64,
    usage_total: u64,
    usage_prompt: u64,
    usage_completion: u64,
    usage_cached: u64,
    usage_cache_write: u64,
    usage_reasoning: u64,
);

/// One landed block: the row, the round it belongs to, and the hash its columns produce.
pub struct WindowBlock {
    /// The round it belongs to; `None` until a round names it.
    pub turn: Option<i64>,
    /// Computed when the block is admitted: what the structure's id claim is matched against.
    pub hash: u64,
    pub block: Block,
}

impl WindowBlock {
    pub fn key(&self) -> i64 {
        self.block.id
    }

    /// One block off a viewport answer: the round it names, or the one the request named.
    fn from_placed(placed: PlacedBlock, asked: Option<i64>) -> Self {
        Self {
            turn: placed.turn.or(asked),
            hash: placed.block.content_key().digest(),
            block: placed.block,
        }
    }
}

/// One piece of the tail: the block, and its hash once the finish signal has arrived.
pub struct LiveBlock {
    pub block: NewBlock,
    /// `Some` the moment the block is known to be finished: it can be claimed by that hash.
    pub hash: Option<u64>,
}

/// What a running tool printed, before its result block lands. Display only.
#[derive(Clone, PartialEq)]
pub struct ToolOutput {
    pub call_id: String,
    pub text: String,
}

/// The tail that has not landed: the blocks the streaming reply has produced so far. Increments
/// arrive one at a time and are appended; a finished piece replaces its block and carries the
/// hash that claims its id. `version` moves only on a real change.
#[derive(Default)]
pub struct LiveTail {
    blocks: Vec<LiveBlock>,
    tool_output: Option<ToolOutput>,
    version: u64,
}

impl LiveTail {
    pub fn blocks(&self) -> &[LiveBlock] {
        &self.blocks
    }

    pub fn tool_output(&self) -> Option<&ToolOutput> {
        self.tool_output.as_ref()
    }

    /// Bumped by every real change: a render side keys its memo on it.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn has_text(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| b.block.content.as_deref().is_some_and(|c| !c.is_empty()))
    }

    /// One piece off the stream: an increment to append, or the finished block to replace. The
    /// finish is where the hash is taken.
    fn feed(&mut self, blocks: Vec<NewBlock>, done: bool) {
        let mut changed = false;
        for block in blocks {
            match self
                .blocks
                .iter_mut()
                .find(|open| same_block(&open.block, &block))
            {
                Some(open) if done => {
                    let hash = Some(block.content_key().digest());
                    if open.block != block || open.hash != hash {
                        open.block = block;
                        open.hash = hash;
                        changed = true;
                    }
                }
                Some(open) => changed |= grow(&mut open.block, &block),
                None => {
                    let hash = done.then(|| block.content_key().digest());
                    self.blocks.push(LiveBlock { block, hash });
                    changed = true;
                }
            }
        }
        if changed {
            self.version = self.version.wrapping_add(1);
        }
    }

    fn set_tool_output(&mut self, tool_output: Option<ToolOutput>) {
        if self.tool_output != tool_output {
            self.version = self.version.wrapping_add(1);
            self.tool_output = tool_output;
        }
    }

    fn clear(&mut self) {
        if !self.blocks.is_empty() {
            self.blocks.clear();
            self.version = self.version.wrapping_add(1);
        }
        self.set_tool_output(None);
    }

    /// Take one block out: a claimed one has landed for real, with its id.
    fn take(&mut self, at: usize) -> Option<LiveBlock> {
        if at >= self.blocks.len() {
            return None;
        }
        let block = self.blocks.remove(at);
        self.version = self.version.wrapping_add(1);
        Some(block)
    }
}

/// Is this piece part of that block? The same rule the write inlet uses: kind, and the tool call's
/// id.
fn same_block(a: &NewBlock, b: &NewBlock) -> bool {
    a.kind == b.kind && a.call_id == b.call_id
}

/// Take one more piece of a block that is still being written: the text grows, everything else is
/// already there. `true` = something moved.
fn grow(open: &mut NewBlock, piece: &NewBlock) -> bool {
    match (&mut open.content, &piece.content) {
        (Some(text), Some(more)) => {
            text.push_str(more);
            true
        }
        (None, Some(more)) => {
            open.content = Some(more.clone());
            true
        }
        _ => false,
    }
}

/// The content cache: `config.ui.session.preloadCache` blocks around the view, plus the unlanded tail.
pub struct Window {
    /// Landed blocks, contiguous and ascending by id.
    window: VecDeque<WindowBlock>,
    tail: LiveTail,
    /// The round the window is writing into; `None` = nothing has been opened yet.
    turn: Option<i64>,
    /// The optimistic interrupt row: it stands until the server's own row lands.
    placeholder: Option<NewBlock>,
    placeholder_text: String,
    /// A page came back empty (or said there was nothing): the latch closes.
    no_more_above: bool,
    no_more_below: bool,
    want_older: Option<i64>,
    want_newer: Option<i64>,
    /// Blocks kept around the view (`config.ui.session.preloadCache`).
    preload: usize,
    /// Bumped by every whole replacement.
    generation: u64,
    /// The blocks the view shows at top and bottom (`None` = on the tail), and the pin.
    top: Option<i64>,
    bottom: Option<i64>,
    pinned: bool,
}

impl Window {
    pub fn new(preload: usize) -> Window {
        Window {
            window: VecDeque::new(),
            tail: LiveTail::default(),
            turn: None,
            placeholder: None,
            placeholder_text: String::new(),
            no_more_above: false,
            no_more_below: false,
            want_older: None,
            want_newer: None,
            preload: preload.max(8),
            generation: 0,
            top: None,
            bottom: None,
            pinned: true,
        }
    }

    pub fn set_budget(&mut self, preload: usize) {
        self.preload = preload.max(8);
    }

    pub fn budget(&self) -> usize {
        self.preload
    }

    /// That row's text (`config.ui.placeholder.interrupted`).
    pub fn set_placeholder_text(&mut self, text: String) {
        self.placeholder_text = text;
    }

    /// Moves only when the whole window is replaced.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn stored(&self) -> &VecDeque<WindowBlock> {
        &self.window
    }

    pub fn tail(&self) -> &LiveTail {
        &self.tail
    }

    pub fn placeholder(&self) -> Option<&NewBlock> {
        self.placeholder.as_ref()
    }

    fn index_of(&self, key: i64) -> Option<usize> {
        self.window.iter().position(|b| b.key() == key)
    }

    /// Replace the transcript whole; `more` = older blocks exist past this frame.
    pub fn replace(&mut self, blocks: Vec<PlacedBlock>, more: bool) {
        self.window = blocks
            .into_iter()
            .map(|p| WindowBlock::from_placed(p, None))
            .collect();
        self.turn = None;
        self.placeholder = None;
        self.no_more_above = !more;
        self.no_more_below = false;
        self.want_older = None;
        self.want_newer = None;
        self.top = None;
        self.bottom = None;
        self.pinned = true;
        self.tail.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Append a landed batch; the tail is cleared (the server resends it without them). A block
    /// already here (claimed from the structure) takes the landed version instead.
    pub fn push_blocks(&mut self, blocks: Vec<Block>) {
        for block in blocks {
            let hash = block.content_key().digest();
            match self.window.iter_mut().find(|b| b.key() == block.id) {
                Some(held) => {
                    held.hash = hash;
                    held.turn = held.turn.or(self.turn);
                    held.block = block;
                }
                None => self.window.push_back(WindowBlock {
                    turn: self.turn,
                    hash,
                    block,
                }),
            }
        }
        self.placeholder = None;
        self.tail.clear();
        self.trim();
    }

    /// Prepend older blocks; an empty page or `more` false closes the upper latch.
    pub fn prepend(&mut self, blocks: Vec<PlacedBlock>, more: bool, asked: Option<i64>) {
        self.want_older = None;
        if blocks.is_empty() || !more {
            self.no_more_above = true;
        }
        for block in blocks.into_iter().rev() {
            self.window
                .push_front(WindowBlock::from_placed(block, asked));
        }
        self.trim();
    }

    /// Append newer blocks; an empty page or `more` false closes the lower latch.
    pub fn append(&mut self, blocks: Vec<PlacedBlock>, more: bool, asked: Option<i64>) {
        self.want_newer = None;
        if blocks.is_empty() || !more {
            self.no_more_below = true;
        }
        for block in blocks {
            self.window
                .push_back(WindowBlock::from_placed(block, asked));
        }
        self.trim();
    }

    pub fn push_live(&mut self, blocks: Vec<NewBlock>, done: bool) {
        self.tail.feed(blocks, done);
    }

    pub fn set_tool_output(&mut self, call_id: String, text: String) {
        self.tail
            .set_tool_output(Some(ToolOutput { call_id, text }));
    }

    /// The round that just opened: the blocks landing from now on belong to it.
    pub fn set_turn(&mut self, turn: i64) {
        self.turn = Some(turn);
    }

    /// The round ended: the tool output goes, and its number is remembered when it is the first
    /// news of it.
    pub fn end_round(&mut self, turn: i64) {
        if self.turn.is_none() {
            self.turn = Some(turn);
        }
        self.tail.set_tool_output(None);
    }

    /// Write a round number onto the blocks that round names, matching by content hash: the
    /// position is not trusted. A finished tail block the round names is admitted with its id.
    pub fn claim(&mut self, turn: i64, blocks: &[TurnBlock]) {
        if blocks.is_empty() {
            return;
        }
        for entry in blocks {
            let (id, hash) = (entry.id, entry.hash);
            let hit = self
                .window
                .iter()
                .position(|b| b.key() == id && b.hash == hash)
                .or_else(|| self.window.iter().position(|b| b.hash == hash));
            if let Some(i) = hit {
                if let Some(block) = self.window.get_mut(i) {
                    block.turn = Some(turn);
                }
                continue;
            }
            // Still in the tail: the finish signal gave it a hash, and the round gives it an id.
            // The block is complete, so it lands here.
            let at = self
                .tail
                .blocks()
                .iter()
                .position(|live| live.hash == Some(hash));
            if let Some(at) = at
                && let Some(live) = self.tail.take(at)
            {
                self.window.push_back(WindowBlock {
                    turn: Some(turn),
                    hash,
                    block: live.block.landed(id),
                });
                self.trim();
            }
        }
    }

    /// Stand the optimistic interrupt row. Nothing is stood when the tail carries a body.
    pub fn stand_placeholder(&mut self) {
        if self.tail.has_text() {
            return;
        }
        self.placeholder = Some(NewBlock {
            kind: crate::server::storage::BlockKind::Assistant,
            content: Some(self.placeholder_text.clone()),
            ..NewBlock::default()
        });
    }

    pub fn clear_placeholder(&mut self) {
        self.placeholder = None;
    }

    /// Remember where the view sits, then say whether to ask for more: `top` / `bottom` are
    /// the ids it shows (`None` = on the tail), and at most one want comes back per call.
    pub fn note_view(
        &mut self,
        top: Option<i64>,
        bottom: Option<i64>,
        pinned: bool,
    ) -> Option<Want> {
        self.top = top;
        self.bottom = bottom;
        self.pinned = pinned;
        let above = top.and_then(|k| self.index_of(k)).unwrap_or(0);
        let below = bottom
            .and_then(|k| self.index_of(k))
            .map(|i| self.window.len().saturating_sub(i + 1))
            .unwrap_or(0);
        if !self.no_more_above && above < self.preload {
            if let Some(edge) = self.window.front() {
                let (before, turn) = (edge.key(), edge.turn);
                self.want_older = Some(before);
                return Some(Want::Older {
                    before,
                    limit: self.preload,
                    turn,
                });
            }
        }
        if !self.no_more_below && below < self.preload {
            if let Some(edge) = self.window.back() {
                let (after, turn) = (edge.key(), edge.turn);
                self.want_newer = Some(after);
                return Some(Want::Newer {
                    after,
                    limit: self.preload,
                    turn,
                });
            }
        }
        None
    }

    /// Evict down to the budget, from the end farthest from the view.
    fn trim(&mut self) {
        let keep = self.preload + SCREEN_BLOCKS;
        if self.pinned {
            // Pinned: the window's end is the stored tail, so only the front may lose blocks.
            while self.window.len() > keep {
                self.window.pop_front();
            }
            return;
        }
        let Some(top) = self.top.and_then(|k| self.index_of(k)) else {
            while self.window.len() > keep {
                self.window.pop_front();
            }
            return;
        };
        let front = top.saturating_sub(self.preload);
        for _ in 0..front {
            self.window.pop_front();
        }
        let top = top - front;
        let back = self
            .bottom
            .and_then(|k| self.index_of(k))
            .unwrap_or(top)
            .max(top);
        let floor = back + self.preload + SCREEN_BLOCKS;
        while self.window.len() > floor {
            self.window.pop_back();
        }
    }
}
