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
    /// Required: this domain has no default.
    pub max_output_tokens: u32,
    pub streaming: Streaming,
    /// Characters a compaction keeps when the caller names no budget.
    pub retain_tail: usize,
}

/// The request half.
pub struct Request {
    options: Options,
    slot: Mutex<Option<(super::contract::Transport, Arc<Client>)>>,
}

impl Request {
    pub fn new(options: Options) -> Result<Self> {
        if options.max_output_tokens == 0 {
            bail!("the output cap is required and must be positive");
        }
        Ok(Self {
            options,
            slot: Mutex::new(None),
        })
    }

    /// The client for one address, rebuilt only when the address changes.
    fn client(&self, transport: &super::contract::Transport) -> Result<Arc<Client>> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((address, client)) = slot.as_ref()
            && address == transport
        {
            return Ok(client.clone());
        }
        let provider = self
            .options
            .providers
            .iter()
            .find(|provider| provider.name == transport.provider)
            .with_context(|| format!("no provider named {}", transport.provider))?;
        let client = Arc::new(Client::new(provider, &transport.model, self.options.retry));
        *slot = Some((transport.clone(), client.clone()));
        Ok(client)
    }

    /// Every message of the session, from the newest compaction marker on.
    fn stored(history: &dyn History, session: i64) -> Result<Vec<Message>> {
        let turns = history.turns(session)?;
        Ok(turns
            .into_iter()
            .flat_map(|turn| turn.pieces.into_iter().map(|piece| piece.message))
            .collect())
    }
}

impl Ask for Request {
    fn lap(&self, input: LapInput) -> Result<LapReply> {
        let client = self.client(&input.transport)?;
        let stored = Self::stored(input.history.as_ref(), input.session)?;
        let req = assembly::build(
            client.model(),
            self.options.max_output_tokens,
            true,
            &input.system,
            &stored,
            &input.tools,
            input.session,
        )?;

        let line = Line::new(input.live.clone(), input.session);
        let mut forward = Forward::new(&line, input.blocks.as_ref(), self.options.streaming);
        let message = client.complete(&req, &mut forward, &line, &input.stop)?;
        forward.finish(message.usage)?;

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
        for call in &input.calls {
            if !input.results.iter().any(|r| r.call_id == call.call_id) {
                bail!("no result for call {}", call.call_id);
            }
        }
        for run in &input.results {
            if !input.calls.iter().any(|c| c.call_id == run.call_id) {
                bail!("a result names no call: {}", run.call_id);
            }
        }

        let mut known: Vec<Reference> = Vec::new();
        let ts = crate::server::platform::time::now_stamp();

        for run in &input.results {
            let (text, attached) = match run.spill.as_ref() {
                Some(spill) => {
                    let (hash, reference) =
                        artifact::resolve(&spill.content, &mut known, input.history.as_ref())?;
                    artifact::materialize(&input.artifacts, &reference, &spill.content)?;
                    let mut text = run.result.clone();
                    text.push_str(&artifact::citation(&reference, &spill.content));
                    (
                        text,
                        Some(Artifact {
                            name: spill.name.clone(),
                            content: spill.content.clone(),
                            hash,
                            reference,
                        }),
                    )
                }
                None => (run.result.clone(), None),
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
            self.options.max_output_tokens,
            true,
            &input.system,
            &plan.region,
            &instruction,
            &input.tools,
            input.session,
        )?;

        let line = Line::new(input.live.clone(), input.session);
        let mut forward = StreamOnly {
            line: &line,
            streaming: self.options.streaming,
            buffered: Vec::new(),
        };
        let message = client.complete(&req, &mut forward, &line, &input.stop)?;
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
            ts: Some(crate::server::platform::time::now_stamp()),
            failure: None,
        }
    }

    fn flush(&mut self) {
        for piece in coalesce(std::mem::take(&mut self.buffered)) {
            self.line.stream(piece);
        }
    }

    fn land(&mut self, mut block: Block) -> Result<()> {
        block.ts = self.ts.clone();
        self.sink.land(block, None)
    }

    /// Close the lap: the held block takes the bill and lands. A write that failed mid-lap surfaces here.
    fn finish(&mut self, usage: Usage) -> Result<()> {
        self.flush();
        if let Some(mut block) = self.pending.take() {
            block.usage = Some(usage);
            if let Err(e) = self.land(block)
                && self.failure.is_none()
            {
                self.failure = Some(e);
            }
        }
        match self.failure.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Output for Forward<'_> {
    fn stream(&mut self, piece: Stream) {
        match self.streaming {
            Streaming::Immediate => self.line.stream(piece),
            Streaming::Buffered => self.buffered.push(piece),
        }
    }

    fn block(&mut self, block: Block) {
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
            (Some(Stream::Text(held)), Stream::Text(more)) => held.push_str(&more),
            (Some(Stream::Thinking(held)), Stream::Thinking(more)) => held.push_str(&more),
            (
                Some(Stream::Arguments {
                    call_id: held_id,
                    text: held,
                }),
                Stream::Arguments {
                    call_id,
                    text: more,
                },
            ) if *held_id == call_id => held.push_str(&more),
            (_, piece) => out.push(piece),
        }
    }
    out
}
