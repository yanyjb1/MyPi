# loop architecture

## Files

```text
mod.rs                  declarations and public exports
session/mod.rs          session types
session/book.rs         Loop, Birth, Message, scheduler, turn start
session/handle.rs       per-session state: birth, live, mailbox, flags
session/live.rs         cwd handle, provider/model, system, tool names, compaction instruction, export dir
session/settings.rs     fence and scrooge
session/mailbox.rs      two queues and the wake wait
session/change.rs       Model and Cwd
session/child.rs        a sub-session's draft and budget
session/source.rs       Source and Furnish over storage's reader
session/export.rs       Export: a full text out of the database as a file
session/history.rs      Transcript: the request half's read port over storage's reader
turn/mod.rs             turn types
turn/cycle.rs           lap loop, mailbox drain, settle
turn/ask.rs             the request boundary, as re-exports
turn/batch.rs           tool-boundary re-exports
turn/env.rs             per-turn tool input
turn/fence.rs           tool-round count
pen/mod.rs              row writers
pen/turn.rs             turn open, append, settle
pen/blocks.rs           the request half's block sink: one lap's rows, flushed as one submission
pen/spill.rs            artifact row
pen/effect.rs           memo, alias, subagent rows
```

The live line is not here: `Event`, `Element`, `Stream`, `Notice` and `ToolEvent` are the request
half's types, re-exported from this domain's root.

## Relations

- `Loop::book` inserts a handle, starts the session scheduler.
- `Loop::deliver` either opens a turn through `pen::turn::open` and starts `turn::cycle::run`, or
  pushes the message into the mailbox when the handle is busy.
- The scheduler waits on `Mailbox::wait_any`; when idle it takes the first queued item (wake first)
  and opens a turn.
- `turn::cycle::run` drains the mailbox, opens `Transcript` as the read port, expands the session's
  tool names through `tools::roster`, asks through `Ask::lap`, runs `Batch::run`, hands the finished
  calls back through `Ask::results` as a `ResultsInput`, appends the effects as rows anchored on
  their result blocks, flushes the lap, books any sub-sessions, and settles through
  `pen::turn::settle`.
- `pen::blocks::Pen` implements `BlockSink`: it holds one lap's block rows (the block's five usage
  numbers plus their sum) and their artifact rows, records each result block's row index by call id,
  and writes the whole lap once. `pen::effect` adds memo and alias rows at those indexes and resolves
  deletions by name inside the session's visible blocks.
- `session::source` answers the tool ports over storage's read plan (`Sheet`, `Cond`, `Query`): the
  filtering, ordering and limit run in SQL, and the caller names columns and values only.
- `session::history::Transcript` answers `History`: the session's list from its newest compaction
  marker on, every block decoded into the request half's messages, and the hash holding a citation.
- `pen::turn::settle` writes `Row::Settle` and, when the first lap reported a prompt, a
  `Row::Baseline`.
- `pen::turn::borrow` writes one borrowed turn per kept original turn, each with its ranges and the
  `borrowed` stop reason.
- `session::change` writes `Row::Switch` or `Row::MoveCwd` and updates the handle; a model switch
  also archives the expanded tool manuals.
- `Loop::new` reads the tool values out of `config`, builds `tools::Engine`, and keeps the same
  `Source` handle for the roster's availability gate.

## Callers

- The API calls `Loop::book`, `deliver`, `interrupt`, `drop`, `change`, `mailbox`.
- The scheduler and turn threads live inside this domain.
- The request half implements `Ask`; the tool half implements `Batch` and its ports are answered by
  `session::source` and `session::export`. Neither touches `Handle`, `Live` or `pen`.
- Storage consumes `Row`s and returns `Landed` ids; the loop never writes SQL.

## Not wired

- The tool half has no `Batch` implementation.
- The API does not construct the loop yet.
