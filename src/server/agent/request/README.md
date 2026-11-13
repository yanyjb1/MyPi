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
(provider name and model id), the system prompt, the tool manuals as raw JSON, the cancel flag, the
session's worker and the live line. `LapReply` carries why the model stopped (`Model` or `User`), the bill, and the calls
it placed (call id, name, arguments verbatim, intent).

`ResultsInput` carries the session id, the history (for citations already handed out), the artifact
directory, and the finished calls. A call without a result fails the whole batch; a result naming no
call fails it too.

`CompactInput` carries the same read end plus retention and the instruction, already rendered from
the template and the focus when the round opened.
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

A stream has no total budget: one lap runs as long as the model keeps talking. The worker child
holds it to the protocol's silence budget (600 s when the host names none) and reports a timeout when
it expires. Cancellation is a kill: the session's worker dies and the reader sees its pipe close. A
closed pipe is the body's end, not a failure; the child sends a marker line before it closes when
the request itself failed.

The history read port has three queries: `messages(session)` — the session's messages, shared, so a
lap never copies them; `turns(session)` — the same list as turns with their interval lists (a turn
has several when other sessions wrote between its batches), used by compaction; and
`holder(reference)` — the content hash that already holds that exact citation, if any.

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

**The host owns the wire, the domain owns the block.** A request body is the standard chat
completions shape and nothing else: model, messages, streaming with its usage, the token ceiling and
the tools. A stored assistant turn's thinking is never replayed, so one history is one size whatever
asks it. The block types are decided here, once, so the loop and the storage never learn a wire
format.

**The loop reads a session once.** The history port hands out one shared snapshot per session, so a
lap never copies it. A restarted process seeds the same bytes from storage, so the prefix cache still
pays.

**Increments and blocks are two different things.** Pieces go to the live line as they arrive; a
block is written once, when it closes. The last block of a reply is held back by one so it can carry
the bill, because a row is written once and never patched. A lap that dies lands the pieces it
already showed as reasoning and assistant blocks without a bill, so the stored turn keeps the
visible causal chain.

**A citation names content.** The same text keeps its citation across sessions and restarts; only a
real collision lengthens it. Retrieval therefore never depends on which block the text landed in.

**Compaction sends only what it replaces.** The retained part stays out of the request, so the
prefix up to the cut is byte-identical to the previous real call.

**Retry belongs to one call.** Only "nothing arrived" and 429 are retried; anything already shown to
a front end is never re-sent, because the front end has already drawn it.

**An interrupt is a kill.** The child that owns the socket belongs to one session, so pressing Esc
kills that session's connection and nothing else. The kernel closes the socket with the child, so
the reader returns at once instead of waiting for the host to speak; a fresh child takes its place
before the next request. A cancelled lap is not a failure: it settles as interrupted, with no
notice, no retry, and no error text.

**A call is only what arrived whole.** A stream cut mid-arguments leaves no call for the tool layer:
the decoder drops the fragment, so the wire is the one that broke its side. That is never a silent
skip: the decoder lands a `SYSTEM:` block naming how many calls never closed and fails the lap, so
what is stored and what the front end heard agree. A delta that names no call slot fails the same
way, because placing it would merge calls into one unparsable argument string.

**Silence is the model's, and only when the upstream says so.** A reply with neither text nor a call
is the model's silence only when the upstream declared a normal ending (`stop`, `end_turn`,
`stop_sequence`, `completed`). Any other ending, or none at all, lands a `SYSTEM:` block naming the
ending and fails the lap. The placeholder therefore means one thing only.

## 3. Unresolved

1. A worker's HTTP status reaches the decoder with its body split across lines; the status survives
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
| `config.server.agent.loop.request.retryTimeout` | how long one attempt waits for the response head, in seconds |

The output cap is not a `config.yaml` key: the caller hands the per-address table in at startup.
