# loop architecture

## Files

```text
mod.rs                  declarations and public exports
session/mod.rs          session types
session/book.rs         Loop, Birth, Message, scheduler, turn start
session/handle.rs       per-session state: birth, live, mailbox, flags
session/live.rs         cwd, provider/model, system, tools, artifacts
session/settings.rs     fence and scrooge
session/transport.rs    provider/model pair
session/mailbox.rs      two queues and the wake wait
session/change.rs       Model and Cwd
session/artifact.rs     the session's artifact directory
turn/mod.rs             turn types
turn/cycle.rs           lap loop, mailbox drain, settle
turn/ask.rs             request boundary and block sink
turn/batch.rs           tool boundary and effects
turn/env.rs             per-turn tool input
turn/fence.rs           tool-round count
pen/mod.rs              row writers
pen/turn.rs             turn open, append, settle
pen/blocks.rs           request block sink
pen/spill.rs            artifact row
pen/birth.rs            child session row
pen/effect.rs           memo, alias, subagent rows
out/mod.rs              live line types
out/element.rs          Element and Event
out/sink.rs             sending end
```

## Relations

- `Loop::book` inserts a handle, starts the session scheduler.
- `Loop::deliver` either opens a turn through `pen::turn::open` and starts `turn::cycle::run`, or
  pushes the message into the mailbox when the handle is busy.
- The scheduler waits on `Mailbox::wait_any`; when idle it takes the first queued item (wake first) and opens a turn.
- `turn::cycle::run` drains the mailbox, asks through `Ask::lap`, runs `Batch::run`, hands results
  back through `Ask::hand_results`, and settles through `pen::turn::settle`.
- `pen::blocks::Pen` implements `BlockSink`: one block row, plus an artifact row that refers to the
  block row in the same submission.
- `pen::turn::settle` writes `Row::Settle` and, when the first lap reported a prompt, a
  `Row::Baseline`.
- `session::change` writes `Row::Switch` or `Row::MoveCwd` and updates the handle.
- `out::Sink` is the live line; the request half writes increments directly, the loop writes tool
  events and notices.

## Callers

- The API calls `Loop::book`, `deliver`, `interrupt`, `drop`, `change`, `mailbox`.
- The scheduler and turn threads live inside this domain.
- Request and tools implement `Ask` and `Batch`; they do not touch `Handle`, `Live` or `pen`.
- Storage consumes `Row`s and returns `Landed` ids; the loop never writes SQL.

## Not wired

- `Ask` and `Batch` implementations are outside this domain.
