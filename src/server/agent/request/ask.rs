//! The three actions the loop asks for: one lap, one batch of results, one compaction.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};

use super::artifact;
use super::assembly;
use super::compact;
use super::contract::{
    Artifact, Ask, Block, BlockSink, CompactInput, CompactReply, History, Kind, LapInput, LapReply,
    Reference, ResultsInput, Retention, Stop, ToolCall,
};
use super::live::{Line, Stream};
use super::message::{Message, Usage};
use super::protocol::Output;
use super::retry::RetryPolicy;
use super::transport::Provider;
use super::transport::client::Client;

/// How much of the stream a front end sees as it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Streaming {
    /// One piece per chunk.
    Immediate,
    /// One piece per block.
    Buffered,
}

/// Everything the request half is configured with at startup.
pub struct Options {
    pub providers: Vec<Provider>,
    pub retry: RetryPolicy,
    pub streaming: Streaming,
    /// Characters a compaction keeps when the caller names no budget.
    pub retain_tail: usize,
}

/// The request half.
pub struct Request {
    options: Options,
    /// One client per address; two sessions on two models never evict each other.
    clients: Mutex<std::collections::BTreeMap<super::contract::Transport, Arc<Client>>>,
}

impl Request {
    pub fn new(options: Options) -> Result<Self> {
        Ok(Self {
            options,
            clients: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    /// The client for one address, built the first time that address is asked for.
    fn client(&self, transport: &super::contract::Transport) -> Result<Arc<Client>> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = clients.get(transport) {
            return Ok(client.clone());
        }
        let provider = self
            .options
            .providers
            .iter()
            .find(|provider| provider.name == transport.provider)
            .with_context(|| format!("no provider named {}", transport.provider))?;
        let client = Arc::new(Client::new(provider, &transport.model, self.options.retry));
        clients.insert(transport.clone(), client.clone());
        Ok(client)
    }

    /// The session's messages, shared; the turn's snapshot is not copied per lap.
    fn stored(history: &dyn History, session: i64) -> Result<Arc<Vec<Message>>> {
        history.messages(session)
    }
}

impl Ask for Request {
    fn lap(&self, input: LapInput) -> Result<LapReply> {
        let client = self.client(&input.transport)?;
        let stored = Self::stored(input.history.as_ref(), input.session)?;
        let req = assembly::build(
            client.model(),
            input.max_output_tokens,
            true,
            &input.system,
            &stored[..],
            input.tools,
            input.tools_json,
            input.session,
        )?;

        let line = Line::new(input.live.clone(), input.session);
        let mut forward = Forward::new(&line, input.blocks.as_ref(), self.options.streaming);
        let outcome = client.complete(&req, &mut forward, &line, &input.stop, &input.worker);
        let message = match outcome {
            Ok(message) => {
                forward.finish(message.usage)?;
                message
            }
            Err(e) => {
                // The front end already saw these pieces; the turn keeps what it showed.
                forward.abandon();
                return Err(e);
            }
        };

        Ok(LapReply {
            stop: if message.interrupted {
                Stop::User
            } else {
                Stop::Model
            },
            usage: message.usage,
            calls: message.calls.iter().map(call_of).collect(),
        })
    }

    fn results(&self, input: ResultsInput, blocks: &dyn BlockSink) -> Result<()> {
        let ResultsInput {
            calls,
            results,
            history,
            ..
        } = input;
        for run in &results {
            if !calls.iter().any(|c| c.call_id == run.call_id) {
                bail!("a result names no call: {}", run.call_id);
            }
        }

        let mut known: Vec<Reference> = Vec::new();
        let ts = crate::server::platform::time::now_stamp();

        for run in results {
            let (text, attached) = match run.spill {
                Some(spill) => {
                    let (hash, reference) =
                        artifact::resolve(&spill.content, &mut known, history.as_ref())?;
                    let mut text = run.result;
                    text.push_str(&artifact::citation(&reference, &spill.content));
                    (
                        text,
                        Some(Artifact {
                            name: spill.name,
                            content: spill.content,
                            hash,
                            reference,
                        }),
                    )
                }
                None => (run.result, None),
            };
            // The cut-short reason is the last line: after the body and its artifact note.
            let text = match run.reason.as_deref() {
                Some(reason) => format!("{text}\n[{reason}]"),
                None => text,
            };

            blocks.land(
                Block {
                    kind: Kind::ToolResult,
                    ts: Some(ts.clone()),
                    call_id: Some(run.call_id.clone()),
                    ok: Some(run.ok),
                    result: Some(text),
                    memo: run.memo.clone(),
                    duration_ms: Some(run.duration_ms),
                    ..Block::default()
                },
                attached,
            )?;
        }
        Ok(())
    }

    fn compact(&self, input: CompactInput) -> Result<CompactReply> {
        let client = self.client(&input.transport)?;
        let turns = input.history.turns(input.session)?;
        let retention = input
            .retention
            .unwrap_or(Retention::Chars(self.options.retain_tail));
        let plan = compact::plan(&turns, retention);
        if plan.region.is_empty() {
            bail!("nothing to compact");
        }

        let instruction =
            compact::render_instruction(&input.instruction, input.focus.as_deref().unwrap_or(""));
        let req = assembly::compaction(
            client.model(),
            input.max_output_tokens,
            true,
            &input.system,
            &plan.region,
            &instruction,
            input.tools,
            input.tools_json,
            input.session,
        )?;

        let line = Line::new(input.live.clone(), input.session);
        let mut forward = StreamOnly {
            line: &line,
            streaming: self.options.streaming,
            buffered: Vec::new(),
        };
        let message = client.complete(&req, &mut forward, &line, &input.stop, &input.worker)?;
        forward.finish();

        if message.interrupted {
            bail!("the compaction was interrupted; the context is untouched");
        }
        if message.content.trim().is_empty() {
            bail!("the summarizing model returned nothing; the context is untouched");
        }
        compact::check_shrink(&message.content, &plan.region)?;

        Ok(CompactReply {
            text: message.content,
            usage: message.usage,
            borrows: plan.borrows,
        })
    }
}

fn call_of(call: &super::message::WireCall) -> ToolCall {
    ToolCall {
        call_id: call.id.clone(),
        name: call.name().to_string(),
        args: call.function.arguments.clone(),
        intent: call.intent(),
    }
}

/// The write side of one lap: pieces go out now, blocks are held one behind so the last one can
/// carry the bill.
struct Forward<'a> {
    line: &'a Line,
    sink: &'a dyn BlockSink,
    streaming: Streaming,
    buffered: Vec<Stream>,
    pending: Option<Block>,
    /// Streamed text no block has claimed yet: it lands if the lap dies.
    text: String,
    /// Streamed thinking no block has claimed yet.
    thinking: String,
    ts: Option<String>,
    failure: Option<anyhow::Error>,
}

impl<'a> Forward<'a> {
    fn new(line: &'a Line, sink: &'a dyn BlockSink, streaming: Streaming) -> Self {
        Self {
            line,
            sink,
            streaming,
            buffered: Vec::new(),
            pending: None,
            text: String::new(),
            thinking: String::new(),
            ts: Some(crate::server::platform::time::now_stamp()),
            failure: None,
        }
    }

    fn flush(&mut self) {
        for piece in coalesce(std::mem::take(&mut self.buffered)) {
            self.line.stream(piece);
        }
    }

    /// The bill rides an increment of the kind the block it was written on carries.
    fn bill(&self, block: &Block, usage: Usage) {
        let piece = match block.kind {
            Kind::Assistant => Stream::Text {
                text: String::new(),
                usage: Some(usage),
            },
            Kind::Reasoning => Stream::Thinking {
                text: String::new(),
                usage: Some(usage),
            },
            _ => Stream::Arguments {
                call_id: block.call_id.clone().unwrap_or_default(),
                name: String::new(),
                text: String::new(),
                usage: Some(usage),
            },
        };
        self.line.stream(piece);
    }

    fn land(&mut self, mut block: Block) -> Result<()> {
        block.ts = self.ts.clone();
        self.sink.land(block, None)
    }

    /// Land the held block; the bill rides it when there is one.
    fn land_pending(&mut self, usage: Option<Usage>) {
        let Some(mut block) = self.pending.take() else {
            return;
        };
        if let Some(usage) = usage {
            block.usage = Some(usage);
            self.bill(&block, usage);
        }
        if let Err(e) = self.land(block)
            && self.failure.is_none()
        {
            self.failure = Some(e);
        }
    }

    /// Close the lap: the held block takes the bill and lands. A write that failed mid-lap surfaces here.
    fn finish(&mut self, usage: Usage) -> Result<()> {
        self.flush();
        self.land_pending(Some(usage));
        match self.failure.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// The lap died: land the held block and the pieces already shown, without a bill.
    fn abandon(&mut self) {
        self.land_pending(None);
        let ts = self.ts.clone();
        if !self.thinking.is_empty() {
            let content = std::mem::take(&mut self.thinking);
            let block = Block {
                kind: Kind::Reasoning,
                content: Some(content),
                ts: ts.clone(),
                ..Block::default()
            };
            let _ = self.sink.land(block, None);
        }
        if !self.text.is_empty() {
            let content = std::mem::take(&mut self.text);
            let block = Block {
                kind: Kind::Assistant,
                content: Some(content),
                ts,
                ..Block::default()
            };
            let _ = self.sink.land(block, None);
        }
    }
}

impl Output for Forward<'_> {
    fn stream(&mut self, piece: Stream) {
        match &piece {
            Stream::Text { text, .. } => self.text.push_str(text),
            Stream::Thinking { text, .. } => self.thinking.push_str(text),
            _ => {}
        }
        match self.streaming {
            Streaming::Immediate => self.line.stream(piece),
            Streaming::Buffered => self.buffered.push(piece),
        }
    }

    fn block(&mut self, block: Block) {
        match block.kind {
            Kind::Assistant => self.text.clear(),
            Kind::Reasoning => self.thinking.clear(),
            _ => {}
        }
        self.flush();
        self.line.stream(Stream::Boundary);
        if self.failure.is_some() {
            return;
        }
        if let Some(previous) = self.pending.replace(block)
            && let Err(e) = self.land(previous)
        {
            self.failure = Some(e);
        }
    }
}

/// A compaction's output: the text streams, the blocks are the loop's to write.
struct StreamOnly<'a> {
    line: &'a Line,
    streaming: Streaming,
    buffered: Vec<Stream>,
}

impl StreamOnly<'_> {
    fn finish(&mut self) {
        for piece in coalesce(std::mem::take(&mut self.buffered)) {
            self.line.stream(piece);
        }
    }
}

impl Output for StreamOnly<'_> {
    fn stream(&mut self, piece: Stream) {
        match self.streaming {
            Streaming::Immediate => self.line.stream(piece),
            Streaming::Buffered => self.buffered.push(piece),
        }
    }

    fn block(&mut self, _block: Block) {
        for piece in coalesce(std::mem::take(&mut self.buffered)) {
            self.line.stream(piece);
        }
        self.line.stream(Stream::Boundary);
    }
}

/// Merge adjacent pieces of the same kind: one block, one piece.
fn coalesce(pieces: Vec<Stream>) -> Vec<Stream> {
    let mut out: Vec<Stream> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match (out.last_mut(), piece) {
            (
                Some(Stream::Text { text: held, usage }),
                Stream::Text {
                    text: more,
                    usage: bill,
                },
            ) => {
                held.push_str(&more);
                if bill.is_some() {
                    *usage = bill;
                }
            }
            (
                Some(Stream::Thinking { text: held, usage }),
                Stream::Thinking {
                    text: more,
                    usage: bill,
                },
            ) => {
                held.push_str(&more);
                if bill.is_some() {
                    *usage = bill;
                }
            }
            (
                Some(Stream::Arguments {
                    call_id: held_id,
                    name: held_name,
                    text: held,
                    usage,
                }),
                Stream::Arguments {
                    call_id,
                    name,
                    text: more,
                    usage: bill,
                },
            ) if *held_id == call_id => {
                if held_name.is_empty() {
                    *held_name = name;
                }
                held.push_str(&more);
                if bill.is_some() {
                    *usage = bill;
                }
            }
            (_, piece) => out.push(piece),
        }
    }
    out
}
