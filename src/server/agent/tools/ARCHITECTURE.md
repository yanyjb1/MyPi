# tools — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and re-exports |
| `contract/mod.rs` | declares the six contract groups |
| `contract/call.rs` | `Batch`, `Cwd` |
| `contract/output.rs` | `ToolResult`, `ToolOutput`, `Effect`, `MemoOp`, `AliasOp` |
| `contract/env.rs` | `Env`, `Defaults`, `Limits`, `ShellEnv`, `Paths`, `Seen`, `Stamp`, the model table (`Models`, `ModelFacts`) and the child roster derived from it (`roster`) |
| `contract/halt.rs` | the one sentence a stopped turn leaves: `INTERRUPTED`, and the answer a tool gives when nothing was written |
| `contract/view.rs` | the `Source`, `Export`, `Child`, `Draft`, `MemoRead` ports, the history shapes, and `MemoScope`, `MemoReadOp`, `MemoVersion`, `MemoView` |
| `contract/job.rs` | the job vocabulary a tool shares with the loop: `JobSpec`, `Schedule`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, `Exit`, `OutputSink`, and the `JobHost` port |
| `engine.rs` | `Engine`: `Batch::run` — env, gates, decode, run, stamp, measure (every tool but `read`), events; a call whose arguments arrived answers, and a stop landing mid-call becomes the result's reason |
| `spec.rs` | `Kind`, `Param`, `schema_of`, `Args`, `Item`, `Tool`, `Run`, `Summary`, `ToolProgress` |
| `summary.rs` | one recorded call's arguments, the one-line history summary every file tool reuses, and the shared text parts: whitespace folding and a summary's own line |
| `registry.rs` | `TOOLS`, `spec`, `roster` |
| `workspace.rs` | `Workspace`, the licensed zones, the per-path write lock, `write_atomic`, `canonicalize_lenient`, `unverbatim` |
| `fs/mod.rs` | declares the fs family |
| `fs/gate.rs` | the process gate: one child run to its end under a deadline, its whole group killed by a stop or a deadline, its two streams read on threads of their own, and where a toolchain program is found; the caller waits on the child's exit alone, and a deadline thread kills the tree to end that wait |
| `fs/skeleton.rs` | the report's skeleton over a line table: the gutter width and a numbered source line, the innermost piece holding a line, the seam drawn around a range |
| `fs/fault.rs` | the batch's audit: one check per unit written into, by the language that owns it, every error and warning landing once — under the edit whose lines it hit, under the write that replaced the file whole, or in the unit's list on the last answer that can take it |
| `fs/change.rs` | `context` (the piece around a changed line, its line numbers and the excerpt), `receipt`, `line_of`, `lines_of` |

## Tool areas

Each area's own files and relations are in its architecture document.

| area | tools | documents |
|---|---|---|
| `fs/read/` | `read` | `fs/read/README.md`, `fs/read/ARCHITECTURE.md` |
| `fs/grep/` | `grep` | `fs/grep/README.md`, `fs/grep/ARCHITECTURE.md` |
| `fs/edit/` | `edit` | `fs/edit/README.md`, `fs/edit/ARCHITECTURE.md` |
| `fs/splice/` | `splice` | `fs/splice/README.md`, `fs/splice/ARCHITECTURE.md` |
| `fs/write/` | `write` | `fs/write/README.md`, `fs/write/ARCHITECTURE.md` |
| `fs/lang/` | the line table every file tool reads | `fs/lang/README.md`, `fs/lang/ARCHITECTURE.md` |
| `cd/` | `cd` | `cd/README.md`, `cd/ARCHITECTURE.md` |
| `shell/` | `shell`, `alias` | `shell/README.md`, `shell/ARCHITECTURE.md` |
| `kairos/` | `kairos` | `kairos/README.md`, `kairos/ARCHITECTURE.md` |
| `mcp/` | `mcp` | `mcp/README.md`, `mcp/ARCHITECTURE.md` |
| `memo/` | `memo`, `todo`, `context` | `memo/README.md`, `memo/ARCHITECTURE.md` |
| `subagent/` | `task`, `final` | `subagent/README.md`, `subagent/ARCHITECTURE.md` |
| `web/` | `web` | `web/README.md`, `web/ARCHITECTURE.md` |
| `search/` | `search` | `search/README.md`, `search/ARCHITECTURE.md` |

## What it talks to

- **`storage`**: not imported here at all; a tool reaches the database only through `Source` and
  `Export`, both answered by the loop.
- **`config`**: not imported by any tool. The loop reads the keys and hands values in, except the
  browser keys, which the web domain reads itself.
- **`platform`**: `time::now_stamp` (the effect stamp) and the local-time parts, `log::warn`,
  `env::temp_dir`, `shell::Shell` (the session's shell: program, one-shot argument shape,
  stored-text path form), `proc` (process group spawn, group signal, tree teardown), `stop::Stop`
  (the session's stop signal and the actions a call registers on it).
- **`request`**: `ToolCall`, `ToolDef`, `Message` for definitions and the context window.
- **`web`** (feature `web`): `WebArgs`, `run` from the `web` and `search` tools; `shutdown` from `main`.
- **Who calls it**: `loop/turn` (one batch at a time, `Batch::run`), `loop/turn/cycle` (`roster`
  for the manuals, and the child list correction), `loop/session/book` (builds `Engine` from
  config), `loop/session/source` (builds a session's job service and MCP bridge on first use).
- **Knows nothing about**: the api doors, the database connection, the front end, rounds, streams.

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
      web, search     ── crate::web (feature `web`)
      memo/*          ── memo, todo, context
  → effect stamping → spill → ToolOutput
```

- `Env` is built once when a session is booked, from `Defaults`; the engine runs it as it stands.
  Its child roster is derived at the same moment: every model flagged `subagent` plus the
  session's own, and a model switch rebuilds it.
- A `JobHost` and a sub-session's `Child` reach a tool through `Source`; the loop attaches both when
  it books a session and takes both off when it drops one. A job is only ever created by the loop.
- The guard is used in two places — the `shell` tool and `alias add` — and judges the expanded
  command in both.
- The MCP bridge is built by `Source::mcp(session, cwd)` on the session's first `mcp` call, from the
  table for its cwd; a later move rescans it. A broken table fails that call, not the turn. An empty
  table still mounts the bridge.
- A child's tool list is corrected once, where the child is spawned: `task` out, `final` in. A main
  session never resolves `final`: `api::post::init` drops it from the list and its manuals.

## Must stay in step

1. `contract::env::Env` and `engine`: every field is filled in one place; a new port is a field here
   and a trait in `contract::view`.
2. `spec::Param` order and each tool's index constants: `Args` reads positionally at the top
   level, and `Args::objects` reads an array item's fields by name against the item table under
   `Kind::Objects`.
3. `schema_of` and `Args::fill`: one `Param` table feeds both; `intent` is added to the flat
   schema in one place and the same fields are read by position in the other.
4. `ToolOutput.effects` and `contract::output::Effect`: the front door stamps `ts`, so a tool must
   never fill it.
5. `contract::job::JobHost` and `loop/job`: the port is the whole surface a tool sees; a new job
   field starts there.
6. `Args::fill`'s field check and every tool's parameter table: a name the table does not hold is
   refused, so a new argument is added to the table or it cannot be passed.
7. `platform::stop::Stop` and its three holders — the session handle, each job's `cancel`, and
   `Env.stop`: one type, one flag, no second way to say stopped.
8. `Pen::drop_calls` and `ask.results`: a call without a result has its request block dropped
   before the commit; a result without a call is refused.
9. `ToolOutput.fail` and `engine`: any reason fails the call, and a call cut short states it in the
   one item it answered; the front door adds one for the stop it observed, and the tool reports its
   own deadline.
10. `fs/fault.rs`'s audit and the crate's own manifest: a file is checked only inside a crate, so a
    file outside any crate is never checked, and one batch gives one check per crate.
11. `lang::table` is the one place a file's structure is read: `change`, `skeleton`, `fault`,
    `read`, `splice`, `grep`, `edit` and `lang::graph` all ask it, so a change to what counts as a
    piece reaches every one of them.
12. `lang::shape` and a language's own relation index (`lang::relations`) are the decisions a
    consumer reads: a tool judges what a piece is through `shape`, and where a name is declared
    through the language's relations, never by cutting a label or a `::`/`.` itself.
13. A sorter answers a table and nothing else: display caps, sorting, folding and prose stay in the
    tool that shows them.

## Not wired yet

1. A `kairos` call in a sub-session fails at call time: a child has no job service, and the roster
   does not screen for it. See `kairos/ARCHITECTURE.md`.
