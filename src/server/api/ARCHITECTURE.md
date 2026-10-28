# Architecture

`src/server/api` is the server's public surface. It is reached by the IPC server, and it reaches
storage, config, the agent loop, and the platform. It does not know any front end.

## Files

| Path | What it holds |
|---|---|
| `mod.rs` | `Api`, module declarations, the public re-exports from `ipc::contract`. |
| `notice.rs` | `Notice`, `NoticeLevel`, `notify`. |
| `call/mod.rs` | `Call`, `answer`, dispatch. |
| `call/ctx.rs` | The read context: config, connection, attached session, live sessions, running rounds. |
| `call/reply.rs` | `Reply`, `SessionReply`, `ToolReply`, `ServerReply`. |
| `call/session/` | `SessionAsk` and the four session queries; `Init` carries the optional initial todo list; shared read helpers. |
| `call/tool/` | `ToolAsk`, memo reads, todo gate/replay, child rows. |
| `call/server/` | `ServerAsk`, config values, completion pools, resume rows. |
| `post/mod.rs` | `Post`, `PostDone`, `ModelId`, `apply`, dispatch. |
| `post/init.rs` | The one write that creates a session and answers with its id. |
| `post/session/` | `SessionWrite` and its files: name, cwd, switch, profile, todo on/off, delete, interrupt, rewind, unwatch. |
| `post/server/` | `ServerWrite`: default model, reload. |
| `post/tool/` | `ToolWrite`, memo add/mod/del. |
| `subscribe/mod.rs` | `Subscribe`, `Opened`, `Retention`, `connect`, `watch`, `submit`, `compact`. |
| `subscribe/event.rs` | Re-exports `Event`, `ContentEvent`, `SessionEvent`, `StartKind`, `TurnUsage` from `ipc::contract`. |
| `subscribe/watch.rs` | Registers the session's full stream. |
| `subscribe/submit.rs` | Opens a round and registers the viewer on its turn. |
| `subscribe/compact.rs` | Opens a compaction round. |
| `driver/mod.rs` | The global state, `Driver`, the entry points, round opening, compaction planning, rewind, interrupt; builds the shared MCP bridge. |
| `driver/sessions.rs` | One `Live` per session, built on demand; job/subagent factories, child specs, and the shared MCP bridge. |
| `driver/child.rs` | The `Spawn` port: run one child session to its end. |
| `driver/running.rs` | The running-round claim and stop flags. |
| `driver/subscriptions.rs` | Viewer streams, the current-turn buffer, queued notices. |
| `driver/relay.rs` | Storage's relay port and the event conversion. |
| `driver/round.rs` | One round thread and its outlet. |
| `driver/wake.rs` | The job wake channel and worker; opens wake rounds with no user message. |
| `driver/compact.rs` | The compaction thread. |

## Public surface

- `api::{Call, Reply, Post, PostDone, Subscribe, Opened, Event, Notice, NoticeLevel, Viewer}`
  and the rest of the wire vocabulary, re-exported from `ipc::contract`.
- `api::notify`.
- `call::answer(cfg, viewer, call)`.
- `post::apply(cfg, viewer, post)`.
- `subscribe::{connect, watch, submit, compact}`.
- `notice::notify`.

`driver` is private. The types inside the group modules are public because the door shapes name
them.

## Who calls it

- The IPC server calls the doors and `subscribe::connect`; it forwards `Event` values to the
  front end.
- Other server domains call `notice::notify` when they have something to say about a session.
- A front end reaches the API only through IPC.

## What it calls

- `convert`: maps `storage::Block`/`NewBlock`/`SessionMeta`/`RoundUsage` and
  `agent::TodoPhase` to their `ipc::contract` counterparts.
- Storage: `Reader`, `Writer`, `Relay`, `Block`, `NewBlock`, `SessionMeta`, `RoundUsage`,
  `Submit`, `Target`, `Cut`.
- Config: `Config`, model resolution, profile resolution, the app config.
- Agent: `Live`, `Settings`, the loop, the tool environment, compaction planning.
- Platform: time, logging, data and runtime directories.
- The command marker types: `Name`, `Cdp`, `Switch`, `Model`, `Profile`.

## What it does not know

- The IPC transport, framing, or connection lifetime.
- Any front end's screen, layout, or rendering.
- Storage's schema internals; it writes its own SQL against the documented tables.
- The config registry; it exposes a wire `ConfigValue`.
- The agent's tool bodies or request encoding.

## Synchronization

- One global `Mutex<Driver>` serializes the doors.
- `Running` is the per-session claim: one round at a time.
- `Sessions` holds one `Live` per session, a writer handle, and the shared MCP bridge; a `Live` is
  taken for a round and put back after. A profile chosen mid-session lands here and is written to the
  session row then.
- `Subscriptions` holds one stream per viewer, the current-turn buffer, and queued notices.
- The writer's relay port is the only source of content events; the round's outlet adds tool
  output and the driver adds started/ended/session events.
- The API's reader connections are independent of the writer.

## Not yet wired

- The config registry does not exist yet; the wire `ConfigValue` lives in `ipc::contract`.
- The compaction summary's turn is not aligned between storage and the API.
- Memo writes do not go through the writer.
