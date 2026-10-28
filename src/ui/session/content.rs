//! The content cache: landed blocks by id, streamed blocks by boundary, in arrival order.

use std::collections::BTreeMap;

use crate::ipc::contract::{Block, BlockKind, NewBlock};

/// Which side of the cache a read extends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Older,
    Newer,
}

/// One upstream packet: live until it has an id, landed after.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    Live(NewBlock),
    Landed(Block),
}

/// One cached block: the upstream kind, and the packet itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Content {
    pub kind: BlockKind,
    pub packet: Packet,
}

impl Content {
    fn live(block: NewBlock) -> Content {
        Content {
            kind: block.kind,
            packet: Packet::Live(block),
        }
    }

    fn landed(block: Block) -> Content {
        Content {
            kind: block.kind,
            packet: Packet::Landed(block),
        }
    }
}

/// A streamed block's identity: what tells a piece from the next block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boundary {
    kind: BlockKind,
    call_id: Option<String>,
}

impl Boundary {
    fn of_live(block: &NewBlock) -> Boundary {
        Boundary {
            kind: block.kind,
            call_id: block.call_id.clone(),
        }
    }

    fn of_landed(block: &Block) -> Boundary {
        Boundary {
            kind: block.kind,
            call_id: block.call_id.clone(),
        }
    }
}

/// What a running tool printed, before its result block lands.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub call_id: String,
    pub text: String,
}

/// The cache itself.
#[derive(Default)]
pub struct ContentCache {
    landed: BTreeMap<i64, Content>,
    live: Vec<(Boundary, Content)>,
    tool_output: Option<ToolOutput>,
    generation: u64,
}

impl ContentCache {
    /// Landed blocks, ascending by id.
    pub fn landed(&self) -> &BTreeMap<i64, Content> {
        &self.landed
    }

    /// Streamed blocks, in arrival order.
    pub fn live(&self) -> impl Iterator<Item = &Content> {
        self.live.iter().map(|(_, content)| content)
    }

    pub fn tool_output(&self) -> Option<&ToolOutput> {
        self.tool_output.as_ref()
    }

    pub fn set_tool_output(&mut self, call_id: String, text: String) {
        self.tool_output = Some(ToolOutput { call_id, text });
    }

    pub fn clear_tool_output(&mut self) {
        self.tool_output = None;
    }

    /// Moves on every whole replacement: a reader's derived cache is invalid across it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Append a streamed piece; `done` replaces the entry with the finished block.
    pub fn push_live(&mut self, block: NewBlock, done: bool) {
        let boundary = Boundary::of_live(&block);
        match self.live.iter_mut().find(|(held, _)| *held == boundary) {
            Some((_, content)) => {
                if done {
                    *content = Content::live(block);
                } else {
                    grow(&mut content.packet, block);
                }
            }
            None => self.live.push((boundary, Content::live(block))),
        }
    }

    /// Replace the whole cache; streamed blocks are gone with the frame that carried them.
    pub fn replace_landed(&mut self, blocks: Vec<Block>) {
        self.landed.clear();
        self.live.clear();
        for block in blocks {
            self.landed.insert(block.id, Content::landed(block));
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// Merge landed blocks into the cache, binding the n-th streamed block to the n-th new id.
    pub fn merge_landed(&mut self, blocks: Vec<Block>) {
        for block in blocks {
            let boundary = Boundary::of_landed(&block);
            let streamed = self.live.iter().position(|(held, _)| *held == boundary);
            if let Some(at) = streamed {
                self.live.remove(at);
            }
            self.landed.insert(block.id, Content::landed(block));
        }
    }

    pub fn clear(&mut self) {
        self.landed.clear();
        self.live.clear();
        self.tool_output = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Take one more piece of a block that is still being written: the text grows.
fn grow(packet: &mut Packet, piece: NewBlock) {
    let Packet::Live(open) = packet else {
        return;
    };
    match (&mut open.content, piece.content) {
        (Some(text), Some(more)) => text.push_str(&more),
        (None, Some(more)) => open.content = Some(more),
        _ => {}
    }
}
