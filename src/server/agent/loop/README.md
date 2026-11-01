# loop

## 1. Design / Schema

One `Loop` owns every session. A session is a handle, a mailbox and a lazy live state. A turn opens
its rows, runs laps, takes the mailboxes and settles.

Public surface:

```text
Loop::new(writer, settings, ask, out, config) -> Loop
Loop::book(Birth) -> Result<SessionId>
Loop::deliver(session, Message) -> Result<Option<TurnId>>
Loop::interrupt(session)
Loop::drop(session)
Loop::change(session, Change) -> Result<()>
Loop::mailbox(session) -> Result<Arc<Mailbox>>

Birth = { session, provider, model, system, tools, compact, meta, cwd }   // tools = names
Message = Submit { prefix, text, suffix } | Compact { retention, focus }
Change = Model { provider, model } | Cwd(path)
```

`ask` is `Arc<dyn request::Ask>`; `out` carries the live line. The loop builds the tool layer from
`config` and keeps the read ports it also implements: `Source` (database reads), `Export` (a full
text out of the database as a file) and `Furnish` (the process-wide MCP bridge). Request shapes come
from `request::contract`; tool shapes come from `agent::tools::contract`; both are re-exported here,
so a caller names one vocabulary. `Retention` is `Chars(n)` or `Blocks(n)`.

`book` only opens the session; the caller has already written the `sessions` row and resolved the
system prompt, the tool names and the compaction instruction. `deliver` opens a turn and returns
its id, or queues the message when the session is busy. `book` starts one scheduler thread per
session; that thread turns a wake into a turn.

`Mailbox` has two queues: wake text from tool results, and user messages. Wake is deterministic and
is written as a `wake` block; user messages may be taken back before the loop reads them. A turn
stops when the user interrupts, or when both queues are empty and the model asks for no tools.

`pen` is the only writer of storage rows. `session::history::Transcript` is the read side: it
implements `request::contract::History` over storage's reader. `turn/ask` is the request boundary,
`turn/batch` the tool boundary. The live line is not this domain's vocabulary; it is the request
half's, and the loop only adds tool events to it.

## 2. Design Intent

- **One session owns its state.** No process-global current session; every lock is per session, and
  a batch runs with the live state taken out of the slot.
- **One write point.** `pen` turns request blocks, tool effects, artifacts, turn openings and
  settlements into `Row`s; no other module builds a `Ref`. A lap's blocks and the effects their
  results produced land in one submission, so an effect's anchor is a row index in that same commit.
- **Each boundary is the other side's.** Request types are declared with the request half, tool
  types with the tool half; the loop re-exports both and imports neither private shape.
- **One scheduler per session.** Wake has to start a turn while the API is idle; the scheduler waits
  on the mailbox and starts only when the slot is free.
- **The user's turns and the tool's wake share one mechanism but not one queue.** User text is
  mutable; wake text is a fact and is never taken back.
- **Baseline is the first lap's prompt.** It is attached to the archive and only lowered; a later
  model switch starts from a different archive and its own baseline.
- **Compaction is a turn mode.** Its first block is the user message carrying the focus, the summary
  lands as an assistant block, its list place carries the marker, and the retained region is
  borrowed by intervals, not copied blocks.

## 3. Unresolved

- The tool half does not implement `Batch` yet, so nothing runs a call.
- The API does not construct the loop yet.

## 4. Config Fields

- `config.server.agent.loop.maxToolCalls` — tool-round fence; false turns it off.
- `config.server.agent.loop.scrooge` — true forbids a mid-session model change.

The output cap is not here: it belongs to the request half's startup options.
