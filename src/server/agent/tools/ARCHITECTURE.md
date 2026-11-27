# tools — architecture

What is here, what each file holds, and who talks to whom; current state only.

## Files

- `mod.rs` — module declarations and re-exports.
- `contract/mod.rs` — declares the six contract groups.
- `contract/call.rs` — `Batch`, `Cwd`.
- `contract/output.rs` — `ToolResult`, `ToolOutput`, `Effect`, `MemoOp`, `AliasOp`.
- `contract/env.rs` — `Env`, `Defaults`, `Limits`, `ShellEnv`, `Paths`, `Seen`, `Stamp`, the model table (`Models`, `ModelFacts`) and the child roster derived from it (`roster`).
- `contract/halt.rs` — the one sentence a stopped turn leaves: `INTERRUPTED`, and the answer a tool gives when nothing was written.
- `contract/view.rs` — the `Source`, `Export`, `Child`, `Draft` and `MemoRead` ports, the history shapes, and `MemoScope`, `MemoReadOp`, `MemoVersion`, `MemoView`.
- `contract/job.rs` — the job vocabulary a tool shares with the loop: `JobSpec`, `Schedule`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, `Exit`, `OutputSink`, and the `JobHost` port.
- `engine.rs` — `Engine::run`, which is `Batch::run`: env, gates, decode, run, stamp and measure every tool but `read`, plus the events; a call whose arguments arrived answers, and a stop landing mid-call becomes the result's reason.
- `spec.rs` — `Kind`, `Param`, `schema_of`, `Args`, `Item`, `Tool`, `Run`, `Summary`, `ToolProgress`.
- `summary.rs` — one recorded call's arguments, the one-line history summary every file tool reuses, and the shared text parts of whitespace folding and a summary's own line.
- `registry.rs` — `TOOLS`, `spec`, `roster`.
- `workspace.rs` — `Workspace`, the licensed zones, the per-path write lock, `write_atomic`, `canonicalize_lenient`, `unverbatim`.
- `fs/mod.rs` — declares the fs family.
- `fs/gate.rs` — the process gate: one child run to its end under a deadline, its whole group killed by a stop or a deadline, its two streams read on threads of their own, and where a toolchain program is found; the caller waits on the child's exit alone, and a deadline thread kills the tree to end that wait.
- `fs/skeleton.rs` — the report's skeleton over a line table: the gutter width and a numbered source line, the innermost piece holding a line, and the seam drawn around a range.
- `fs/fault.rs` — the batch's audit: one check per unit written into, by the language that owns it, with every error and warning landing once, whether under the edit whose lines it hit, under the write that replaced the file whole, or in the unit's list on the last answer that can take it.
- `fs/change.rs` — `context` for the piece around a changed line with its line numbers and the excerpt, plus `receipt`, `line_of` and `lines_of`.

## Tool areas

Each area's own files and relations are in its architecture document.

1. `fs/read/` — `read`, documented in `fs/read/README.md` and `fs/read/ARCHITECTURE.md`.
2. `fs/grep/` — `grep`, documented in `fs/grep/README.md` and `fs/grep/ARCHITECTURE.md`.
3. `fs/edit/` — `edit`, documented in `fs/edit/README.md` and `fs/edit/ARCHITECTURE.md`.
4. `fs/splice/` — `splice`, documented in `fs/splice/README.md` and `fs/splice/ARCHITECTURE.md`.
5. `fs/write/` — `write`, documented in `fs/write/README.md` and `fs/write/ARCHITECTURE.md`.
6. `fs/lang/` — the line table every file tool reads, documented in `fs/lang/README.md` and `fs/lang/ARCHITECTURE.md`.
7. `cd/` — `cd`, documented in `cd/README.md` and `cd/ARCHITECTURE.md`.
8. `shell/` — `shell` and `alias`, documented in `shell/README.md` and `shell/ARCHITECTURE.md`.
9. `kairos/` — `kairos`, documented in `kairos/README.md` and `kairos/ARCHITECTURE.md`.
10. `mcp/` — `mcp`, documented in `mcp/README.md` and `mcp/ARCHITECTURE.md`.
11. `memo/` — `memo`, `todo` and `context`, documented in `memo/README.md` and `memo/ARCHITECTURE.md`.
12. `subagent/` — `task` and `final`, documented in `subagent/README.md` and `subagent/ARCHITECTURE.md`.
13. `web/` — `web`, documented in `web/README.md` and `web/ARCHITECTURE.md`.
14. `search/` — `search`, documented in `search/README.md` and `search/ARCHITECTURE.md`.

## What it talks to

- **`storage`**: not imported here at all, because a tool reaches the database only through `Source` and `Export`, both answered by the loop.
- **`config`**: the value types `Rules` and `BrowserOptions`, which the loop reads once and hands in through `Env`; no tool opens the file.
- **`platform`**: `time::now_stamp` for the effect stamp and the local-time parts, `log::warn`, `env::temp_dir`, `shell::Shell` for the session's shell with its program, one-shot argument shape and stored-text path form, `proc` for process group spawn, group signal and tree teardown, and `stop::Stop` for the session's stop signal and the actions a call registers on it.
- **`request`**: `ToolCall`, `ToolDef` and `Message` for definitions and the context window.
- **`browser`** (feature `browser`): `Setup`, `WebArgs` and `run`, used by the `web` and `search` tools, with `shutdown` called from `main`.
- **Who calls it**: `loop/turn` runs one batch at a time through `Batch::run`, `loop/turn/cycle` calls `roster` for the manuals and the child list correction, `loop/session/book` builds `Engine` from config, and `loop/session/source` builds a session's job service and MCP bridge on first use.
- **Knows nothing about**: the api doors, the database connection, the front end, rounds and streams.

## Inside

```text
Engine::run
  → registry::spec              and env.allows
  → Args::fill
  → Tool::run
      fs/*            ── the file tools and the line table they all read
      shell/*         ── admit, guard, one command line
      kairos          ── Source::jobs (a JobHost) ── loop/job
      subagent/*      ── template, answer name ── Effect::Child
      mcp             ── mcp::sync ── mcp::stdio
      web, search     ── crate::browser (feature `browser`)
      memo/*          ── memo, todo, context
  → effect stamping → spill → ToolOutput
```

- `Env` is built once when a session is booked, from `Defaults`, and the engine runs it as it stands; its child roster is derived at the same moment from every model flagged `subagent` plus the session's own, and a model switch rebuilds it.
- A `JobHost` and a sub-session's `Child` reach a tool through `Source`, and the loop attaches both when it books a session and takes both off when it drops one; a job is only ever created by the loop.
- The guard is used in two places, the `shell` tool and `alias add`, and it judges the expanded command in both.
- The MCP bridge is built by `Source::mcp(session, cwd)` on the session's first `mcp` call, from the table for its cwd, and a later move rescans it; a broken table fails that call rather than the turn, and an empty table still mounts the bridge.
- A child's tool list is corrected once where the child is spawned, with `task` out and `final` in; a main session never resolves `final`, because `api::post::init` drops it from the list and its manuals.

## Must stay in step

1. `contract::env::Env` and `engine`, since every field is filled in one place and a new port is a field here and a trait in `contract::view`.
2. `spec::Param` order and each tool's index constants, because `Args` reads positionally at the top level while `Args::objects` reads an array item's fields by name against the item table under `Kind::Objects`.
3. `schema_of` and `Args::fill`, since one `Param` table feeds both, `intent` is added to the flat schema in one place, and the same fields are read by position in the other.
4. `ToolOutput.effects` and `contract::output::Effect`, because the front door stamps `ts` and a tool must never fill it.
5. `contract::job::JobHost` and `loop/job`, since the port is the whole surface a tool sees and a new job field starts there.
6. `Args::fill`'s field check and every tool's parameter table, because a name the table does not hold is refused and a new argument is added to the table or it cannot be passed.
7. `platform::stop::Stop` and its three holders, the session handle, each job's `cancel` and `Env.stop`, which are one type with one flag and no second way to say stopped.
8. `Pen::drop_calls` and `ask.results`, because a call without a result has its request block dropped before the commit and a result without a call is refused.
9. `ToolOutput.fail` and `engine`, because any reason fails the call and a call cut short states it in the one item it answered, where the front door adds one for the stop it observed and the tool reports its own deadline.
10. The audit in `fs/fault.rs` and the crate's own manifest, because a file is checked only inside a crate, so a file outside any crate is never checked and one batch gives one check per crate.
11. `lang::table`, the one place a file's structure is read by `change`, `skeleton`, `fault`, `read`, `splice`, `grep`, `edit` and `lang::graph`, so a change to what counts as a piece reaches every one of them.
12. `lang::shape` and a language's own relation index in `lang::relations`, which are the decisions a consumer reads, since a tool judges what a piece is through `shape` and where a name is declared through the language's relations rather than by cutting a label or a `::`/`.` itself.
13. A sorter answers a table and nothing else, because display caps, sorting, folding and prose stay in the tool that shows them.

## Not wired yet

1. A `kairos` call in a sub-session fails at call time, because a child has no job service and the roster does not screen for it; see `kairos/ARCHITECTURE.md`.