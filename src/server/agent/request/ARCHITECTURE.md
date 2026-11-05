# request architecture

## Files

| file | holds |
|---|---|
| `request.rs` | module declarations and re-exports; no logic |
| `ask.rs` | `Request`: the three actions, the options it is built with, the forwarder that routes pieces and blocks |
| `contract.rs` | every shape the loop fills or reads: `Ask`, `LapInput`, `LapReply`, `ResultsInput`, `CompactInput`, `CompactReply`, `Borrow`, `Stop`, `Retention`, `Transport`, `ToolCall`, `ToolResult`, `Spill`, `Block`, `Kind`, `Artifact`, `BlockSink`, `History`, `TurnView`, `Piece`, `Reference`, `ResultsInput` |
| `live.rs` | the live line: `Event`, `Live`, `Stream`, `Notice`, `NoticeLevel`, `ToolEvent`, `Line` |
| `message.rs` | the stored `Message`, the wire `WireMessage`, `WireCall`, `ToolDef`, `Usage`, `AssistantMessage`, and the two fixed reply texts |
| `transcript.rs` | stored messages to wire messages: the only translation |
| `assembly.rs` | one request's body: system prompt, history, tool manuals |
| `compact.rs` | the cut, the shrink check, the instruction rendering |
| `artifact.rs` | citations, the artifact file, the citation line |
| `retry.rs` | policy, failure classes, the notice one failure produces |
| `protocol/mod.rs` | `ProtocolId`, `Protocol`, `Endpoint`, `ProtocolRequest`, `WireRequest`, `Output` |
| `protocol/blocks.rs` | block constructors |
| `protocol/sse/` | `read` (one raw line), `frame` (`data:` frames and the worker's error line), `event` (`event:` + `data:` frames) |
| `protocol/stream.rs` | the stream loop: cancel, the silent-connection budgets, `data:` or `event:` frames, how it ended |
| `protocol/stitch.rs` | tool calls stitched out of stream deltas |
| `protocol/openai/` | the OpenAI family: `completion`, `responses`, `deepseek`, each with its own body and decode |
| `protocol/openai/completion/` | the completion dialect: its compatibility table, the text-embedded call scanner, the template-markup filter |
| `protocol/anthropic/` | the Messages line |
| `transport/mod.rs` | `Provider`, and sending one call to either the worker or a direct connection |
| `transport/client.rs` | one provider connection: its agent, retry loop, cancel flag |
| `transport/worker.rs` | `SessionWorker` (one child per session: spawn, kill, replace, in-flight) and the child's line protocol |

## Relations

- `ask::Request::lap` asks `History::turns`, flattens it, builds with `assembly::build`, and hands the
  call to `Client::complete`.
- `Client::complete` asks its `Protocol` for a `WireRequest`, sends it through `transport::send`, and
  decodes the reader through the same `Protocol`. It owns retry and the notices one failure produces;
  the kill belongs to the session's worker, not to this loop.
- A decoder writes pieces and closed blocks into `Output`. `Forward` forwards pieces to the live line
  and lands blocks through `BlockSink`, one behind, so the last one can carry the usage.
- `ask::Request::results` resolves citations through `History::holder`, writes each artifact, and
  lands one block per result.
- `ask::Request::compact` plans the cut with `compact::plan`, builds with `assembly::compaction`,
  streams the summary, and checks the shrink before returning.
- `transport::send` asks the session's worker for a child; a direct connection when the child could
  not start. It never retries.
- `protocol::blocks` builds every block; no protocol builds one by hand.
- The request half opens no database, reads no configuration file, and never names a tool.

## Callers

- The loop calls the three actions and implements `History` and `BlockSink`.
- The configuration supplies `Options`: the provider table, the retry policy, the output cap, the
  streaming granularity, and the default retention.
- The loop owns each session's `SessionWorker` on its handle: `LapInput` / `CompactInput` carry it,
  and `Handle::interrupt` is what kills the child.
- Storage is reached by the loop, never from here.

## Not wired

- A session that could not start its child falls back to a direct connection; cancellation there
  still waits on the reader, and the retry path tries the child again.
