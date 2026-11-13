# tools — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and re-exports |
| `contract/mod.rs` | declares the five contract groups |
| `contract/call.rs` | `Batch`, `Cwd` |
| `contract/output.rs` | `ToolResult`, `ToolOutput`, `Effect`, `MemoOp`, `AliasOp` |
| `contract/env.rs` | `Env`, `Defaults`, `Limits`, `BashEnv`, `Paths` |
| `contract/view.rs` | the `Source`, `Export`, `Child`, `Draft`, `MemoRead` ports, the history shapes, and `MemoScope`, `MemoReadOp`, `MemoVersion`, `MemoView` |
| `contract/job.rs` | the job vocabulary a tool shares with the loop: `JobSpec`, `Schedule`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, `Exit`, `OutputSink`, and the `JobHost` port |
| `engine.rs` | `Engine`: `Batch::run` — env, gates, decode, run, stamp, measure, events; the stop that lands mid-call becomes the result's reason |
| `spec.rs` | `Kind`, `Param`, `schema_of`, `Args`, `Item`, `Tool`, `Run`, `Summary`, `ToolProgress` |
| `summary.rs` | one recorded call's arguments, and the clip every tool's history summary reuses |
| `registry.rs` | `TOOLS`, `spec`, `roster` |
| `workspace.rs` | `Workspace`, the licensed zones, the per-path write lock, `write_atomic`, `canonicalize_lenient` |
| `bash/mod.rs` | declares the bash family |
| `bash/bash/mod.rs` | declares the command tool, the admission gate, the segment scanner, the guard and the runner |
| `bash/bash/tool.rs` | the `bash` tool |
| `bash/bash/admit.rs` | `admit`: aliases → `#ref` → guard, and the line it hands the runner |
| `bash/bash/segment.rs` | the separator set and the segment heads; the guard and the alias expansion share both |
| `bash/bash/guard.rs` | the command classifier: `normalize`, `classify`, `Verdict`, `RuleHit` |
| `bash/bash/shell.rs` | one command line: `run`, `run_to`, `OutputSink`, `Exit`, `StopFlag`; registers the stop action, spawns and signals through `platform::proc` |
| `bash/read/{mod,tool}.rs` | `read`: the file array, the shared line and byte caps, the per-file header |
| `bash/write/{mod,tool}.rs` | `write` |
| `bash/edit/{mod,tool}.rs` | `edit` with `exact` and `mass`; `exact_text`, `mass_text`, `parse_range`, the one-line receipt |
| `bash/cd/{mod,tool}.rs` | `cd` |
| `bash/alias/{mod,tool}.rs` | `alias` add/list/remove; the config layer is read-only here |
| `bash/context/{mod,tool}.rs` | `context`: `query` through `Source::history`, `show` through `Source::block`, long listings to `Export::dir` |
| `kairos/mod.rs` | declares the kairos modules |
| `kairos/tool.rs` | the `kairos` tool manual and parameters |
| `kairos/service.rs` | `kairos` operation dispatch and rendering |
| `kairos/command.rs` | a line admitted at call time as a `JobRunner`: only `run_to` is left |
| `kairos/text.rs` | the text `JobRunner`: nothing to run, so only the clock makes it due |
| `kairos/duration.rs` | relative duration parse (`10min`, `1h30m`) |
| `mcp/mod.rs` | the `mcp` tool: list and call |
| `mcp/bridge.rs` | `McpBridge`, `ServerInfo`, `ToolInfo`, `McpResult`, `McpContent` |
| `mcp/setup.rs` | `mcp.json` → the server table, read for the session's cwd; the project's file replaces the global one |
| `mcp/sync.rs` | `SyncMcpBridge`: one lazy stdio server per name; a rescan re-reads the table and drops the pool when the table or the directory changed |
| `mcp/stdio.rs` | one stdio server process: spawn, JSON-RPC, pending map, stderr tail, shutdown |
| `memo/mod.rs` | declares the memo family |
| `memo/memo/{mod,tool}.rs` | the `memo` tool: writes as effects, reads through `MemoRead` |
| `memo/todo/{mod,tool}.rs` | the `todo` tool: reads the anchor, replays, applies, writes the memo |
| `memo/todo/replay.rs` | the pure replay: a call sequence → the list, anchored on the newest reset |
| `memo/todo/ops.rs` | the three todo operations (`set`, `tick`, `view`) and their batch apply |
| `memo/todo/state.rs` | `TodoItem`, `counts` |
| `subagent/mod.rs` | declarations and re-exports (`TASK`, `FINAL`), and `project_dir` |
| `subagent/task.rs` | the `task` tool: the call's project folder, template, variables, answer name, the `Child` effect |
| `subagent/final.rs` | the `final` tool: the child's draft, its edits, the one submit |
| `subagent/template.rs` | template lookup (project, then global, then the named path), front matter, `{{var}}` rendering, roster subset checks, the stem |
| `subagent/answer.rs` | the answer file: the first free name inside the project's `answers` folder, taken against the names one call already handed out, and the one stamped write |
| `web/mod.rs` | the `web` tool entry: arguments to `WebArgs`, one call to the domain; built only with the `web` feature |

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
- **`web`**: `WebArgs`, `run` from the `web` tool; `shutdown` from `main`.
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
      bash/*          ── shell, file tools, alias, context
      kairos          ── Source::jobs (a JobHost) ── loop/job
      task            ── template, answer name ── Effect::Child
      final           ── Source::child (a Child) ── draft, answer file
      mcp             ── mcp::sync ── mcp::stdio
      web             ── crate::web
      memo/todo       ── Source, and the pure todo replay
  → effect stamping → spill → ToolOutput
```

- `Env` is built once when a session is booked, from `Defaults`; the engine runs it as it stands.
- A `JobHost` and a sub-session's `Child` reach a tool through `Source`; the loop attaches both when
  it books a session and takes both off when it drops one. A job is only ever created by the loop.
- The guard is used in two places — the `bash` tool and `alias add` — and judges the expanded
  command in both.
- The MCP bridge is built by `Source::mcp(session, cwd)` on the session's first `mcp` call, from the
  table for its cwd; a later move rescans it. A broken table fails that call, not the turn. An empty
  table still mounts the bridge.
- A child's tool list is corrected once, where the child is spawned: `task` out, `final` in.

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
5. `bash` and `kairos`: every command goes through `admit`; the guard runs once, on the final text.
6. `licensed_zones` and `guard::escapes_zone`: two independent zone laws; changing one does not
   change the other.
7. `subagent::template` and the parent roster: a template may only name tools the parent has.
8. `contract::job::JobHost` and `loop/job`: the port is the whole surface a tool sees; a new job
   field starts there.
9. `web` feature and two places: the `mod.rs` declaration and the `TOOLS` row.
10. `mcp::setup::table` and `loop/session/source`: the loop supplies the global path and the session's
    cwd; the domain never decides where the session stands.
11. The web domain's config keys and `src/web/README.md`: it reads them itself, so a rename here is
    a rename there.
12. `platform::stop::Stop` and its three holders — the session handle, each job's `cancel`, and
    `Env.stop`: one type, one flag, no second way to say stopped.
13. `Pen::drop_calls` and `ask.results`: a call without a result has its request block dropped
    before the commit; a result without a call is refused.
14. `ToolOutput.fail` and `engine`: any reason makes the result `ok: false`; the front door adds
    one for the stop it observed, and the tool reports its own deadline.

## Not wired yet

1. A `kairos` call in a sub-session fails at call time: a child has no job service, and the roster
   does not screen for it.
