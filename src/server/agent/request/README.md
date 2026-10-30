# request

## 1. Design / Schema

`request` talks to one model host. It turns a session's stored history into one HTTP call, decodes
the reply as it arrives, cuts it into blocks, and hands each block to the loop's write end. It also
turns a batch of tool results into result blocks, and it runs compaction.

The loop sees three actions:

```text
lap(LapInput) -> LapReply
results(ResultsInput, &dyn BlockSink)
compact(CompactInput) -> CompactReply
```

`LapInput` carries the session id, the history read port, the block write port, the transport
(provider name and model id), the system prompt, the tool manuals as raw JSON, the cancel flag and
the live line. `LapReply` carries why the model stopped (`Model` or `User`), the bill, and the calls
it placed (call id, name, arguments verbatim, intent).

`ResultsInput` carries the session id, the history (for citations already handed out), the artifact
directory, and the finished calls. A call without a result fails the whole batch; a result naming no
call fails it too.

`CompactInput` carries the same read end plus retention, focus and the instruction template.
`CompactReply` carries the summary, its bill, and the intervals that stay verbatim, grouped by
original turn, in block order.

Blocks have four kinds: assistant text, reasoning, one per tool call, one per tool result. A reply's
last block carries that reply's usage, whatever kind it is. The block also carries the landing
timestamp.

The live line is one channel with three kinds, `Stream`, `Notice` and `ToolEvent`. Request produces
the first two:

```text
Stream = Text | Thinking | Arguments { call_id, text } | Boundary
Notice = { level, text }
```

`ToolEvent` belongs to the loop.

The history read port has two queries: `turns(session)` — from the newest compaction marker on, in
list order, each turn carrying its interval list (a turn has several when other sessions wrote
between its batches) and its messages keyed by block id; and `holder(reference)` — the content hash that already
holds that exact citation, if any.

A tool result whose text exceeds the tools' own budget arrives with its full text. Request writes it
under the artifact directory, appends a citation to the visible part, and hands the artifact to the
write end with the block. A citation is the full FNV-1a 64 of the content as hex, shortened to 4, 6,
8, 10, 12, 14 or 16 digits: the same content always gets the same citation, and a prefix another
content already uses is lengthened.

Compaction cuts the history by retention, character budget or block count, newest block first; the
newest block always stays. Everything before the cut is the region the summary replaces; the
retained part is not sent. The request is the system prompt, the tool manuals, the region, and the
instruction last. A summary not smaller than its region is an error and leaves the context untouched.

## 2. Design Intent

**The host owns the wire, the domain owns the block.** Compatibility switches decide field names,
reasoning replay and streaming quirks. The block types are decided here, once, so the loop and the
storage never learn a wire format.

**Every lap rebuilds from storage.** Nothing about a conversation is cached here, so a restarted
process sends the same bytes and the prefix cache still pays.

**Increments and blocks are two different things.** Pieces go to the live line as they arrive; a
block is written once, when it closes. The last block of a reply is held back by one so it can carry
the bill, because a row is written once and never patched.

**A citation names content.** The same text keeps its citation across sessions and restarts; only a
real collision lengthens it. Retrieval therefore never depends on which block the text landed in.

**Compaction sends only what it replaces.** The retained part stays out of the request, so the
prefix up to the cut is byte-identical to the previous real call.

**Retry belongs to one call.** Only "nothing arrived" and 429 are retried; anything already shown to
a front end is never re-sent, because the front end has already drawn it.

## 3. Unresolved

1. The loop does not implement `History` or `BlockSink` yet, so this domain is not wired.
2. A worker's HTTP status reaches the decoder with its body split across lines; the status survives
   and the body may arrive empty.

## 4. Config Fields

Consumers ask the configuration for these; the request half receives the values, it never reads a
file.

| key | meaning |
|---|---|
| `config.server.request.streaming` | `immediate` sends one piece per chunk, `buffered` one piece per block |
| `config.server.request.compact.retainTail` | characters a compaction keeps when the caller names no budget |
| `config.server.agent.loop.request.maxRetries` | retries after the first attempt |
| `config.server.agent.loop.request.retryInterval` | fixed wait between attempts, in seconds |
| `config.server.agent.loop.request.retryTimeout` | how long one attempt waits for the first byte, in seconds |

The output cap is not a configuration key: the caller must hand one in at startup.
