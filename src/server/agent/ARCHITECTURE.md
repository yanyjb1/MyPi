# agent architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations: `loop`, `request`, `tools` |
| `loop/mod.rs` | the round: `Live`, `Settings`, `Turn`, `Stop`, `Ended`, `Outlet`, `ToolEvent`, and `run` |
| `loop/round.rs` | one lap: send, decode, run the calls, and the outlet adapter |
| `loop/driver.rs` | per-session tool state: cwd, tool budget, and `run_calls` |
| `loop/env.rs` | `build` for the tool `Env`, the `context` window, and the artifact handle |
| `loop/artifacts.rs` | citation → temp file |
| `loop/outlet.rs` | `Outlet`, `ToolEvent`, and the retry adapter |
| `loop/budget.rs` | the per-round tool fence |
| `request.rs` | the request half's face: `start`, `hand_results`, `rebuild`, `ToolRun` |
| `request/message.rs` | the stored `Message` and the wire `WireMessage` / `Context`, plus `AssistantMessage`, `ToolCall`, `ToolDef`, `Usage`, `StopReason` |
| `request/content.rs` | `RequestContent`, `Answer`, `BlockSink`, `RetrySink`, and `RequestMeta` |
| `request/retry.rs` | retry policy, failure classes, retry notices |
| `request/assembly/mod.rs` | `build`: the one request assembly point and the system prompt entry |
| `request/assembly/history.rs` | stored messages → protocol messages, and tool-call grouping |
| `request/assembly/compact.rs` | compaction plan, replay context, and the prepared summary request |
| `request/protocols/mod.rs` | `ProtocolId`, `Protocol`, `ProtocolRequest`, `WireRequest` |
| `request/protocols/openai/` | Chat Completions, DeepSeek, Responses, shared SSE decode, in-band scanner, markup filter |
| `request/protocols/anthropic/` | Messages request and SSE decode |
| `request/transport/mod.rs` | `send`: one `WireRequest` out, one line reader back |
| `request/transport/client.rs` | the provider connection, protocol, retry loop, worker slot |
| `request/transport/worker.rs` | the long-lived HTTP subprocess |
| `tools/mod.rs` | the declarations and what this folder exports |
| `tools/spec.rs` | `Tool`, `Param`, `Kind`, `Args`, `ToolOutput`, `Spill`, `ToolProgress`, `Env`, `Inbox`, `BashEnv`, `Artifacts`, `MemoRead` |
| `tools/registry.rs` | `TOOLS` and `roster` |
| `tools/executor.rs` | name lookup, profile gate, argument filling, output spill |
| `tools/workspace.rs` | the working zone and path resolution |
| `tools/bash/` | `bash`, `read`, `write`, `edit`, `cd`, `alias`, `context` |
| `tools/memo/` | `memo` and `todo` |
| `tools/kairos/` | the `kairos` tool and its command/text jobs |
| `tools/jobs/` | `JobFactory`/`JobService`, scheduler, runner, wake, output, cron, duration |
| `tools/subagent/` | `task`, `final`, templates, child spawn, runner, draft |
| `tools/mcp/` | the static `mcp` tool and its `McpBridge` seam |
| `tools/web/` | the `web` tool, compiled only with the `web` feature |

## The public surface

| item | what it does |
|---|---|
| `loop::run(w, r, live, turn, out)` | one round: from the start signal to the settle |
| `loop::{Live, Settings, Turn, Stop, Ended, Outlet, ToolEvent}` | what a caller holds and what it hears |
| `request::start(w, r, client, content, retry, cancel)` | one request, start to cut |
| `request::hand_results(w, session, reply, ran)` | cut one lap's tool results into blocks |
| `request::rebuild(r, session)` | the history as messages (the `context` tool's window) |
| `request::ToolRun` | one finished tool call, as the request side sees it |
| `request::content::{RequestContent, Answer, BlockSink, RetrySink}` | a request's input, output and increment sink |
| `request::message::*` | both shape sets, and the tool/usage types built on them |
| `request::assembly::{build, compact, history}` | the outgoing side; `compact` is the prepared summarization request |
| `request::protocols::{ProtocolId, Protocol, ProtocolRequest, WireRequest}` | the protocol seam |
| `request::transport::{send, client::Client, worker}` | the wire |
| `tools::{execute, roster, TOOLS}` | run a call, the filtered table, the table |
| `tools::{Env, Inbox, Window, Workspace, Artifacts, ToolOutput, Spill, ToolProgress, Args, Kind, Param, Run, Tool}` | the tool layer's vocabulary |
| `tools::TodoPhase` | one phase of the todo list |
| `tools::jobs::{JobFactory, JobService, JobSpec, JobRunner, JobCtx, JobOutcome, Wake, WakeSink}` | the per-session async framework and its API-side wake port |
| `tools::subagent::{SubagentFactory, Subagent, ChildSpec, Spawn}` | the child service and the API-side run port |
| `tools::{McpBridge, SyncMcpBridge, McpResult, McpContent, ServerInfo, ToolInfo}` | the MCP bridge seam and its default stdio implementation |

Everything else in this folder is private to its module.

## Relationships

```text
  api ──loop::run(writer, connection, live, turn, outlet)──► loop
  api ──compact::plan + compact::compact + Client::complete──► request
  config ──values────────────────────────────────────────────► request

  loop ──request::start / hand_results / rebuild──► request
  loop ──tools::{execute, roster}─────────────────► tools
  tools ──Artifacts::path_for─────────────────────► the session's state (api side)
  api ──JobFactory::make / SubagentFactory::make──► tools
  api ──SubagentFactory::install / ChildSpawn─────► tools::subagent
  api ──SyncMcpBridge::new / Sessions::new───────► tools::mcp
  tools::jobs ──WakeSink::wake────────────────────► api wake worker
  tools::subagent ──Spawn::run────────────────────► api child round

  loop, request ──Writer::push / own SQL on a handed-in connection──► storage
  tools ──nothing───────────────────────────────────────────────────►  (no database at all)
  everyone ──clock, log, temp directory, environment──► platform
  everyone ──values, never files──► config
```

- **Who calls this domain**: the api. Nothing else under `server/`.
- **The compaction is driven from the api**: it gathers the transcript, plans the cut, and calls
  `compact::compact` with a callback that uses the session's `Client::complete`.
- **Who this domain calls**: storage (the writer, and reads on the connection it is handed), config
  (values), platform (the clock, the log, the temp directory), and — for a child — the api's
  `Spawn` port. It never reaches the api's doors.
- **What it does not know**: a front end, the command line, the api's doors, and any table beyond
  the column lists `storage::decode` hands out.
- **Threads**: one round thread per round (started by the api), one scheduler thread per session,
  one worker per running job, and — when a provider connection carries a subprocess — that
  subprocess. A child round runs on the agent job's worker thread, not on the parent's round thread.
  Each live MCP server adds one process, one stdout reader thread, and one stderr reader thread.

## Inside one round

```text
  loop::run ──► env::build ──► Env ──► tools::execute (one call at a time)
   │                            ▲            │
   │                            └── ToolOutput { text, memo, spill }
   ▼
  request::start ──► assembly ──► Protocol::request ──► transport ──► the provider
   │  ◄── Protocol::decode (streamed, increments go to the sink)
   ▼
  the cut ─┐
  request::hand_results ─┼──► storage Writer: Submit::Blocks / Request / RoundEnd
  the round's end ───────┘
        storage Relay ──► api ──► whoever is watching
```

- **Who reads and writes the database**: the loop and the request half push to the writer (a reply's
  blocks, a lap's tool results, the request archive, the round's end) and read through the connection
  the api hands in — the history, the session row, an artifact row — using storage's own column lists
  and decoders. This domain owns no table and writes no SQL of its own beyond those reads. The tools
  touch no database at all.

## Parts that must stay in step

1. `assembly::build` and `assembly::compact::replay_context` — the system prompt goes in at the
   front in both; one rule, two entry points, both in `assembly`.
2. `request::hand_results` and `storage::shape`'s block columns — the cut decides what fills them,
   `Message::to_block` is the only conversion.
3. `tools::Spill` and `storage::Spill` — two shapes for one thing; `hand_results` maps them.
4. `config`'s `OutputLimits` and `Env.limits` — the `read` tool and the spill gate read the same
   numbers.
5. `ProtocolId` and the config's model entries: a provider's protocol name is one word in both.
6. `history::to_wire`'s grouping and `Message::ToolRequest`'s `first` / `text` — replay has to rebuild
   the message the model actually sent, not N separate calls.
7. The citation: storage spells it (`base36`), and both readers here parse it — `bash`'s
   `rewrite_refs` and `context`'s anchor lookup.
8. `tools::spec`'s parameter tables and `schema_of` — the manual and the argument filling are two
   readers of one declaration.
9. `tools::subagent::spawn::ChildSpec` and the api's `Sessions::build` — the child row, the rendered
   system, the template tools and the budget must all be in place before the child's first request.
10. `tools::subagent::draft::path` and `Call::Children` — the final path is derived from the parent
    root and the child id; no table stores it.

## Not wired yet

1. `config.server.request.streaming` has no reader **inside this domain**: `src/ipc/stream.rs` reads
   it.
2. The placeholder scheme is not implemented: the built-in empty and interrupted texts are hard-coded
   and written into the block.
3. The fork mode of a rewind is still a `todo!()` on the api side.
4. The TUI does not consume `Call::Children` yet: child sessions can be listed and watched at the api
   boundary, but no child tree or final card is drawn.
