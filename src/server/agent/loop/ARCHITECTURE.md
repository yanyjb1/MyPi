# loop architecture

## Files

```text
mod.rs                  declarations and public exports
job/mod.rs              JobService (the JobHost), Factory, Values
job/job.rs              Id, TaskState, Spec, Job, ScheduleEntry, State
job/scheduler.rs        one thread per session: pool, clocks, skips, prune, wake
job/output.rs           the per-run temp file, an OutputSink; the per-session directory
job/cron.rs             five-field cron parse and next
job/wake.rs             Wake, WakeSink, the command wake texts
session/mod.rs          session types
session/book.rs         Loop, Birth, Message, scheduler, turn start, bind, book_child, unbook, forget
session/handle.rs       per-session state: the two configs, the cwd handle, mailbox, transcript, flags
session/config.rs       the request side: model, endpoint, output cap, system, manuals, compact
session/jobs.rs         Slots (the wake router), the scheduler's values from config
session/agent.rs        AgentRunner: books a child, waits for its turn, drops it
session/settings.rs     fence and scrooge
session/mailbox.rs      the wake queue, the one user slot, the wake wait
session/change.rs       Model and Cwd
session/child.rs        a sub-session's draft, budget, answer path, submit flag, nudges
session/source.rs       Source over storage's reader, plus the handles it builds on demand
session/export.rs       Export: a full text out of the database as a file
session/history.rs      Transcript: the session's cached history and the request half's read port
turn/mod.rs             turn types (ask, cycle, fence)
turn/cycle.rs           lap loop, mailbox drain, child nudge, settle
turn/ask.rs             the request boundary, as re-exports
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

- `Loop::book` reads the session's transcript, inserts a handle and starts the turn scheduler.
  `Source::jobs` builds that session's job service on its first job, and `Source::mcp` builds its
  MCP bridge on its first `mcp` call (rescanning when the session moved). `Loop::drop` reverses all
  three through `unbook`, which also wakes the parked scheduler: `interrupt` alone does nothing to
  an idle session, and a scheduler left parked keeps the handle alive.
- `Loop::deliver` refuses a sub-session and a busy session, and otherwise opens a turn through
  `pen::turn::open` and starts `turn::cycle::run`.
- `Loop::write_mail` and `Loop::clear_mail` are the slot's only writers from outside.
- `Loop::forget` reads a session's history again; the api calls it after a rewind.
- The scheduler waits on `Mailbox::wait_any`; when idle it takes a wake text first, then the slot,
  and opens a turn. A turn the slot opens is announced on the live line so the api can name it.
- `start_turn` appends the turn's opening message to the session's transcript; `turn::cycle::run`
  takes the wake queue into the running turn, appends each wake, expands the session's tool names
  through `tools::roster`, asks through `Ask::lap` with the session's shared snapshot, runs
  `Batch::run`, hands the finished calls back through `Ask::results`, appends the landed blocks and
  the effects anchored on their result blocks, flushes the lap, and settles through
  `pen::turn::settle`. A lap that dies returns to `cycle::run`, which flushes its blocks and settles
  the turn before the error goes up. A compaction reloads the transcript after it settles.
- A child effect lands as an archive row, a session row and the result block in one submission; the
  session row points at the child's own archive. After the commit the loop attaches the child's
  `State` (draft, budget, answer path) and starts an `agent` job; `session::agent::AgentRunner`
  books the child, waits on the handle's settle bell until the turn ends (or the task's deadline
  passes, which interrupts it), reads the submit flag, drops the child through `unbook` and reports
  the answer path in the wake.
- A sub-session round that ends with an empty mailbox and no submit gets one more user block, at
  most three times, before the turn is allowed to end.
- `job::JobService` answers `tools::contract::job::JobHost`; `session::jobs::Slots` holds the session
  table, routes each finished job's wake into its session's mailbox, and `Values` is the four config
  keys.
- `pen::blocks::Pen` implements `BlockSink`: it holds one lap's block rows (the block's five usage
  numbers plus their sum) and their artifact rows, records each result block's row index by call id,
  and writes the whole lap once. `pen::effect` adds memo and alias rows at those indexes and resolves
  deletions by name inside the session's visible blocks.
- `session::source` answers the tool ports over storage's read plan (`Sheet`, `Cond`, `Query`): the
  filtering, ordering and limit run in SQL, and the caller names columns and values only.
- `session::history::Transcript` answers `History`: it holds the session's list from its newest
  compaction marker on as one flat message list, hands it out as a shared snapshot, appends each
  landed block, and reads the list again on `reload`. `turns` reads the database for compaction, and
  `holder` holds a citation's hash.
- `pen::turn::settle` writes `Row::Settle` and, when the session's first turn reported a prompt, a
  `Row::Baseline`. Later turns and compaction turns never report one.
- `pen::turn::borrow` writes one borrowed turn per kept original turn, each with its ranges and the
  `borrowed` stop reason.
- `session::change` writes `Row::Switch` or `Row::MoveCwd` and updates the handle. A change never
  touches the archive: a session keeps the one it was created with.
- `Loop::new` reads the tool values out of `config`, builds `tools::Engine`, and keeps the same
  `Source` handle for the roster's availability gate.

## Callers

- The API calls `Loop::book`, `deliver`, `interrupt`, `drop`, `forget`, `change`, `write_mail`,
  `clear_mail`.
- The scheduler, turn and job threads live inside this domain.
- The request half implements `Ask`; the tool half implements `Batch` and its ports are answered by
  `session::source` and `session::export`. Neither touches `Handle`, the session's config, or `pen`.
- Storage consumes `Row`s and returns `Landed` ids; the loop never writes SQL.

## Not wired

- Same-batch calls that have not landed are invisible to `Source`.
