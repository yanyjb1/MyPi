# tools — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and re-exports |
| `contract.rs` | the outward shapes: `Batch`, `BatchInput`, `ToolResult`, `Effect`, `ToolOutput`, `Env`, `Cwd`, `Limits`, `BashEnv`, `Browser`, `Paths`, and the `Source`, `Furnish`, `Export`, `Child`, `Draft`, `MemoRead` ports |
| `contract/job.rs` | the job vocabulary a tool shares with the loop: `JobSpec`, `Schedule`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, and the `JobHost` port |
| `engine.rs` | `Engine`: `Batch::run` — env, gates, decode, run, stamp, measure, events |
| `spec.rs` | `Kind`, `Param`, `schema_of`, `Args`, `Tool`, `Run`, `Summary`, `ToolProgress` |
| `summary.rs` | one recorded call's arguments, and the clip every tool's history summary reuses |
| `registry.rs` | `TOOLS`, `spec`, `roster`, `available`, `Availability` |
| `workspace.rs` | `Workspace`, the licensed zones, the per-path write lock, `write_atomic`, `canonicalize_lenient` |
| `bash/mod.rs` | declares the bash family |
| `bash/summary.rs` | one recorded call's arguments, and the summary clip every bash-family tool reuses |
| `bash/bash/mod.rs` | declares the command tool, its guard, its runner and the shared ref rewriting |
| `bash/bash/tool.rs` | the `bash` tool, `expand_alias`, `rewrite_refs` |
| `bash/bash/guard.rs` | the command classifier: `normalize`, `classify`, `Verdict`, `RuleHit` |
| `bash/bash/shell.rs` | one command: `run`, `run_to`, `OutputSink`, `Exit`, `StopFlag`; spawn and stop through `platform::proc` |
| `bash/read/{mod,tool}.rs` | `read` |
| `bash/write/{mod,tool}.rs` | `write` |
| `bash/edit/{mod,tool}.rs` | `edit` with `exact` and `mass`; `exact_text`, `mass_text` |
| `bash/cd/{mod,tool}.rs` | `cd` |
| `bash/alias/{mod,tool}.rs` | `alias` add/list/remove |
| `bash/context/{mod,tool}.rs` | `context`: `query` through `Source::history`, `show` through `Source::block`, long listings to `Export::dir` |
| `kairos/mod.rs` | the `kairos` tool manual and parameters |
| `kairos/service.rs` | `kairos` operation dispatch and rendering |
| `kairos/command.rs` | a frozen shell command as a `JobRunner`: alias, guard, artifacts, `run_to` |
| `kairos/text.rs` | the text `JobRunner`: nothing to run, so only the clock makes it due |
| `kairos/duration.rs` | relative duration parse (`10min`, `1h30m`) |
| `mcp/mod.rs` | the `mcp` tool: list and call |
| `mcp/bridge.rs` | `McpBridge`, `ServerInfo`, `ToolInfo`, `McpResult`, `McpContent` |
| `mcp/sync.rs` | `SyncMcpBridge`: one lazy stdio server per name |
| `mcp/stdio.rs` | one stdio server process: spawn, JSON-RPC, pending map, stderr tail, shutdown |
| `memo/mod.rs` | declares the memo family |
| `memo/memo/{mod,tool}.rs` | the `memo` tool: writes as effects, reads through `MemoRead` |
| `memo/todo/{mod,tool}.rs` | the `todo` tool: reads the switch, replays, applies, writes the memo |
| `memo/todo/replay.rs` | the pure replay: a call sequence → the list, plus the switch-aware read |
| `memo/todo/ops.rs` | the three todo operations (`set`, `tick`, `view`) and their batch apply |
| `memo/todo/state.rs` | `TodoItem`, `counts` |
| `subagent/mod.rs` | declarations and re-exports (`TASK`, `FINAL`) |
| `subagent/task.rs` | the `task` tool: template, variables, answer name, the `Child` effect |
| `subagent/final.rs` | the `final` tool: the child's draft, its edits, the one submit |
| `subagent/template.rs` | template lookup, front matter, `{{var}}` rendering, roster subset checks, the stem |
| `subagent/answer.rs` | the answer file: the first free name under the answers folder, and the one stamped write |
| `web/mod.rs` | the `web` tool entry and `close_session`; built only with the `web` feature |

## What it talks to

- **`storage`**: not imported here at all; a tool reaches the database only through `Source` and
  `Export`, both answered by the loop.
- **`config`**: not imported by any tool; the loop reads the keys and hands values in. The two
  exceptions are `mcp/sync.rs` and `mcp/stdio.rs`, which still name deleted config types.
- **`platform`**: `time::now_stamp` (the effect stamp) and the local-time parts, `log::warn`,
  `env::temp_dir`, `shell::Shell` (the session's shell: program, one-shot argument shape,
  stored-text read form), `proc` (process group spawn, group signal, tree teardown).
- **`request`**: `ToolCall`, `ToolDef`, `Message` for definitions and the context window.
- **`web`**: `WebArgs`, `run`, `close_session`, only from the `web` tool.
- **Who calls it**: `loop/turn` (one batch at a time, `Batch::run`), `loop/session/book` (builds
  `Engine` from config), `apps`/`api` for `roster` and `close_web_session`.
- **Knows nothing about**: the api doors, the database connection, the front end, rounds, streams.

## Inside

```text
Engine::run
  → registry::{spec, available}   and env.allows
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

- `Env` is built once per batch by `engine`, from the `BatchInput` the loop passes.
- A `JobHost` and a sub-session's `Child` reach a tool through `Source`; the loop attaches both when
  it books a session and takes both off when it drops one. A job is only ever created by the loop.
- The guard is used in two places — the `bash` tool and `alias add` — and judges the expanded
  command in both.

## Must stay in step

1. `registry::TOOLS` and `available`: a gate that hides a row must match the runtime check inside
   the tool.
2. `spec::Param` order and each tool's index constants: `Args` reads positionally.
3. `schema_of` and `Args::fill`: one `Param` table feeds both.
4. `ToolOutput.effects` and `contract::Effect`: the front door stamps `ts`, so a tool must never
   fill it.
5. `bash` and `kairos/command.rs`: both run alias → `#ref` → guard → shell, in that order.
6. `Workspace::zones` and `guard::escapes_zone`: two independent zone laws; changing one does not
   change the other.
7. `subagent::template` and the parent roster: a template may only name tools the parent has.
8. `contract::job::JobHost` and `loop/job`: the port is the whole surface a tool sees; a new job
   field starts there.
9. `web` feature and two places: the `mod.rs` declaration and the `TOOLS` row.

## Not wired yet

1. `web` does not build: it binds the removed `spec::Env` and `Inbox`.
2. `mcp` does not build: `mod.rs` binds the same removed `Env`, and `sync.rs` with `stdio.rs` name
   `config::McpConfig` and `config::McpServerConfig`, which are gone. The bridge must take its
   servers from `mcp.json`, and `Furnish::mcp` answers `None` until it does.
3. The todo replay's front-end handoff is not built: the loop side reads the switch and the last
   off block, the api door that returns the list is still to come.
