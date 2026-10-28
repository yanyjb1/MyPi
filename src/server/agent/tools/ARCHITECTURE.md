# tools — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and re-exports |
| `spec.rs` | `Kind`, `Param`, `schema_of`, `Args`, `ToolOutput`, `Spill`, `ToolProgress`, `Tool`, `Run`, `Env`, `Inbox`, `BashEnv`, the `Artifacts` and `MemoRead` ports |
| `registry.rs` | `TOOLS`, `spec`, `roster`, `available` |
| `executor.rs` | `execute`: lookup, gates, decode, run, effect stamping, spill |
| `workspace.rs` | `Workspace`, the licensed zones, the per-path write lock, `write_atomic`, `canonicalize_lenient` |
| `bash/mod.rs` | declares the bash family |
| `bash/bash/mod.rs` | the `bash` tool, `expand_alias`, `rewrite_refs` |
| `bash/bash/guard.rs` | the command classifier: `normalize`, `classify`, `Verdict`, `RuleHit` |
| `bash/bash/shell.rs` | one command: `run`, `run_to`, `OutputSink`, `Exit`, `StopFlag`, process-group signal |
| `bash/read/mod.rs` | `read` |
| `bash/write/mod.rs` | `write` |
| `bash/edit/mod.rs` | `edit` with `exact` and `mass`; `exact_text`, `mass_text` |
| `bash/cd/mod.rs` | `cd` |
| `bash/alias/mod.rs` | `alias` add/list/remove |
| `bash/context/mod.rs` | `context`, and the `Window` an `Inbox` carries |
| `jobs/mod.rs` | `JobFactory`, `JobService`, re-exports |
| `jobs/job.rs` | `Id`, `JobSpec`, `Schedule`, `TaskState`, `Job`, `Receipt`, `Row`, `Status` |
| `jobs/scheduler.rs` | the per-session scheduler thread, workers, pruning, wake merging |
| `jobs/cron.rs` | five-field cron parse and `next` |
| `jobs/duration.rs` | relative duration parse |
| `jobs/runner.rs` | `JobCtx`, `JobOutcome`, `JobRunner` |
| `jobs/output.rs` | the job temp directory and `OutputFile`, an `OutputSink` |
| `jobs/wake.rs` | `Wake`, `WakeSink`, wake text |
| `kairos/mod.rs` | the `kairos` tool manual and parameters |
| `kairos/service.rs` | `kairos` operation dispatch and rendering |
| `kairos/command.rs` | a frozen shell command as a `JobRunner`: alias, guard, artifacts, `run_to` |
| `mcp/mod.rs` | the `mcp` tool: list and call |
| `mcp/bridge.rs` | `McpBridge`, `ServerInfo`, `ToolInfo`, `McpResult`, `McpContent` |
| `mcp/sync.rs` | `SyncMcpBridge`: one lazy stdio server per name |
| `mcp/stdio.rs` | one stdio server process: spawn, JSON-RPC, pending map, stderr tail, shutdown |
| `memo/mod.rs` | declares the memo family |
| `memo/memo/mod.rs` | the `memo` tool: writes as effects, reads through `MemoRead` |
| `memo/todo/mod.rs` | the `todo` tool |
| `memo/todo/ops.rs` | the nine todo operations and their batch apply |
| `memo/todo/state.rs` | `TodoTask`, `TodoStatus`, `TodoPhase`, `counts` |
| `subagent/mod.rs` | declarations and re-exports (`TASK`, `FINAL`, `Subagent`, `SubagentFactory`, `ChildSpec`, `Spawn`, `START`) |
| `subagent/task.rs` | the `task` tool |
| `subagent/final.rs` | the `final` tool |
| `subagent/service.rs` | `SubagentFactory`, `Subagent`: templates, child rows, reservation, `final_op` |
| `subagent/spawn.rs` | `Spawn`, `ChildSpec`, `create` |
| `subagent/runner.rs` | `AgentRunner`: a `JobRunner` that spawns a child and finishes its draft |
| `subagent/template.rs` | template lookup, front matter, `{{var}}` rendering, roster subset checks |
| `subagent/draft.rs` | the child draft files and `Drafts` |
| `subagent/contract.rs` | the fixed child system text and start text |
| `web/mod.rs` | the `web` tool entry and `close_session`; built only with the `web` feature |

## What it talks to

- **`storage`**: `Effect`, `EffectKind`, `AliasOp`, `MemoScope`, `Submit`, `Writer`, base36 ids.
  Only through values a tool returns or a port the API hands in.
- **`config`**: `ToolsConfig`, `OutputLimits`, `BrowserConfig`, `KairosConfig`, `McpConfig`,
  `SubagentConfig`. Read by the caller, never by a tool.
- **`platform`**: `time::now_stamp`, `log::warn`, `env::temp_dir`, `env::config_dir`.
- **`request`**: `ToolCall`, `ToolDef`, `Message` for definitions and the context window.
- **`web`**: `WebArgs`, `run`, `close_session`, only from the `web` tool.
- **Who calls it**: `loop/driver.rs` (one call at a time, `execute`), `loop/env.rs` (builds `Env`),
  `api/driver` (builds the job, MCP and subagent services), `api/call/tool/todo.rs` (replays calls
  on a fresh `Env`), `api/post/session/delete.rs` (`close_web_session`).
- **Knows nothing about**: the api doors, the database connection, the front end, rounds, streams.

## Inside

```text
executor::execute
  → registry::{spec, available}   and env.allows
  → Args::fill
  → Tool::run
      bash/*          ── shell, file tools, alias, context
      kairos          ── jobs
      task/final      ── subagent ── jobs, storage::Writer
      mcp             ── mcp::sync ── mcp::stdio
      web             ── crate::web
      memo/todo       ── Inbox ports
  → effect stamping → spill → ToolOutput
```

- `Env` is built once per round (`loop/env.rs::build`) and lent to calls in order.
- `JobService` and `Subagent` are built per session by `api/driver` and reach a tool through
  `Inbox`.
- The guard is used in two places — the `bash` tool and `alias add` — and judges the expanded
  command in both.

## Must stay in step

1. `registry::TOOLS` and `available`: a gate that hides a row must match the runtime check inside
   the tool, and `executor::unknown` must not name tools that are not rows.
2. `spec::Param` order and each tool's index constants: `Args` reads positionally.
3. `schema_of` and `Args::fill`: one `Param` table feeds both.
4. `ToolOutput.effects` and `storage::Effect`: the front door stamps `call_id` and `ts`, so a tool
   must never fill them.
5. `bash` and `kairos/command.rs`: both run alias → `#ref` → guard → shell, in that order.
6. `Workspace::zones` and `guard::escapes_zone`: two independent zone laws; changing one does not
   change the other.
7. `subagent::template` and the parent roster: a template may only name tools the parent has.
8. `jobs::JobService` and `subagent::service`: a child holds a reserved slot until it activates.
9. `web` feature and three places: the `mod.rs` declaration, the `TOOLS` row, and
   `close_web_session` in `api/post/session/delete.rs`.

## Not wired yet

1. Stored aliases never reach a new round: nothing builds `Env.aliases` from `storage::aliases`.
2. `Inbox.todo` is written by the `todo` tool but never read back into `Live.todo`; only
   `Sessions::build` and a rewind assign it.
3. `ChildSpec.parent` is set and never read.
4. `JobService::session`, `JobService::sink` and `SubagentFactory::drafts` have no caller.
