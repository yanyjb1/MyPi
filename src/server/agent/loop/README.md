# loop

## 1. Design / Schema

One `Loop` owns every session. A session is a handle, a mailbox and a lazy live state. A turn opens
its rows, runs laps, takes the mailboxes and settles.

Public surface:

```text
Loop::new(writer, settings, ask, batch, out) -> Loop
Loop::book(Birth) -> Result<SessionId>
Loop::deliver(session, Message) -> Result<Option<TurnId>>
Loop::interrupt(session)
Loop::drop(session)
Loop::change(session, Change) -> Result<()>
Loop::mailbox(session) -> Result<Arc<Mailbox>>

Birth = { session, provider, model, system, tools, meta, cwd }
Message = Submit { prefix, text, suffix } | Compact { retention, focus }
Change = Model { provider, model } | Cwd(path)
```

`book` only opens the session; the caller has already written the `sessions` row. `deliver` opens a
turn and returns its id, or queues the message when the session is busy. `book` starts one scheduler
thread per session; that thread turns a wake into a turn.

`Mailbox` has two queues: wake text from tool results, and user messages. Wake is deterministic and
is written as a `wake` block; user messages may be taken back before the loop reads them. A turn
stops when the user interrupts, or when both queues are empty and the model asks for no tools.

`pen` is the only writer of storage rows. `out` is the live line: increments, boundaries, tool
events and notices, each addressed by session. `turn/ask` is the request boundary; `turn/batch` is
the tool boundary.

## 2. Design Intent

- **One session owns its state.** No process-global current session; every lock is per session, and
  a batch runs with the live state taken out of the slot.
- **One write point.** `pen` turns blocks, effects, artifacts, turn openings and settlements into
  `Row`s; no other module builds a `Ref`.
- **One scheduler per session.** Wake has to start a turn while the API is idle; the scheduler waits
  on the mailbox and starts only when the slot is free.
- **The user's turns and the tool's wake share one mechanism but not one queue.** User text is
  mutable; wake text is a fact and is never taken back.
- **Baseline is the first lap's prompt.** It is attached to the archive and only lowered; a later
  model switch starts from a different archive and its own baseline.
- **Compaction is a turn mode.** Its first block is a user message, its list place carries the
  marker, and the retained region is borrowed by intervals, not copied blocks.

## 3. Unresolved

- `turn/ask` and `turn/batch` are the domain boundary; the request and tool implementations are
  outside this domain.
- `turn/ask::BlockSink` takes `request::Block`; `pen` maps it into the storage block row.

## 4. Config Fields

- `config.server.agent.loop.maxToolCalls` — tool-round fence; false turns it off.
- `config.server.agent.loop.scrooge` — true forbids a mid-session model change.
- `config.server.agent.tools.*` — output limits and browser values, passed through `Settings`.
