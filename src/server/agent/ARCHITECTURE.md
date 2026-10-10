# agent architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations: `loop`, `request`, `tools` |
| `loop/mod.rs` | the round: `Live`, `Settings`, `Turn`, `Stop`, `Ended`, `Outlet`, `ToolEvent`, and `run` |
| `loop/round.rs` | one lap: `once` (send, decode, run the calls), `calls`, and `Sink` (the outlet adapter) |
| `loop/env.rs` | `build` / `restore` for the tool `Env`, the `context` window, and `Arts` — the artifact handle that materializes a citation into a temp file |
| `request.rs` | the request half's face: `start`, `hand_results`, `rebuild`, `ToolRun`, and the cut of a reply (`new_blocks`) |
| `request/message.rs` | the two shape sets: `Message` (the stored five) and `WireMessage` / `Context` (the protocol), plus `AssistantMessage`, `ToolCall`, `ToolDef`, `Usage`, `NoticeLevel` |
| `request/content.rs` | `RequestContent`, `Answer`, `AnswerSink`, and `RequestMeta` (re-exported from storage) |
| `request/assembly/mod.rs` | `build`: the only place a request body is built, and where the system prompt enters |
| `request/assembly/history.rs` | stored messages → protocol messages (`to_wire`, `Rebuild`), and `system_message` to read the prompt back |
| `request/assembly/body.rs` | the body JSON, and each host's say about every field |
| `request/assembly/compact.rs` | the compaction's cut (`plan`, `Retention`, `KeptTurn`) and its prepared request (`replay_context`, `compact`, `summary_turn`, `summary_message`) |
| `request/assembly/compat/{mod,types,rules,defaults,engine}.rs` | the per-host differences, resolved into one `Resolved` |
| `request/inbound/mod.rs` | the request path: `streamed` |
| `request/inbound/decode.rs` | the streamed response → `AssistantMessage` |
| `request/inbound/response.rs` | the wire shapes and the tool-call stitch |
| `request/inbound/inband.rs` | the scanner for hosts that carry tool calls inside text |
| `request/inbound/markup.rs` | the template-markup filter |
| `request/transport/mod.rs` | `send`: hand a body out, get a line reader back |
| `request/transport/client.rs` | `Client`: the provider connection, its compat, the retry decisions, the subprocess slot; `PROTOCOL` |
| `request/transport/worker.rs` | the subprocess a long-lived connection is killed through |
| `tools/mod.rs` | the declarations, and what this folder exports |
| `tools/spec.rs` | what a tool is: `Tool`, `Param`, `Kind`, `Args`, `ToolOutput`, `Spill`, `ToolProgress`, and what it gets: `Env`, `Inbox`, `Window`, `Artifacts`, `BashEnv` |
| `tools/registry.rs` | `TOOLS` (one row per tool, in the order the model reads them) and `roster` |
| `tools/executor.rs` | the front door: name lookup, profile gate, arguments, and the output gate |
| `tools/workspace.rs` | the working zone, and how a path is resolved inside it |
| `tools/{read,edit,write,cd,bash,mass_edit,todo,context}/` | one tool per folder (`bash` adds `guard` and `shell`; `todo` adds `ops` and `state`) |
| `tools/{fetch,browser,search}/` | the web tools, compiled in only with the `web` feature |

## The public surface

| item | what it does |
|---|---|
| `loop::run(w, r, live, turn, out)` | one round: from the start signal to the settle |
| `loop::{Live, Settings, Turn, Stop, Ended, Outlet, ToolEvent}` | what a caller holds and what it hears |
| `request::start(w, r, transport, content, sink, cancel)` | one request, start to cut |
| `request::hand_results(w, session, reply, ran)` | cut one lap's tool results into blocks |
| `request::rebuild(r, session)` | the history as messages (the `context` tool's window) |
| `request::ToolRun` | one finished tool call, as the request side sees it |
| `request::content::{RequestContent, Answer, AnswerSink}` | a request's input, output and increment sink |
| `request::message::*` | both shape sets, and the tool/usage types built on them |
| `request::assembly::{build, compact, history}` | the outgoing side; `compact` is the prepared summarization request |
| `request::inbound::{streamed, decode}` | the incoming side |
| `request::transport::{send, client::Client, worker}` | the wire |
| `tools::{execute, roster, TOOLS}` | run a call, the filtered table, the table |
| `tools::{Env, Inbox, Window, Workspace, Artifacts, ToolOutput, Spill, ToolProgress, Args, Kind, Param, Run, Tool}` | the tool layer's vocabulary |
| `tools::TodoPhase` | one phase of the todo list |

Everything else in this folder is private to its module.

## Relationships

```text
  api ──loop::run(writer, connection, live, turn, outlet)──► loop
  api ──compact::{plan, compact} + transport::send + inbound::decode──► request
  config ──transport::client::PROTOCOL────────────────────────────────► request

  loop ──request::start / hand_results / rebuild──► request
  loop ──tools::{execute, roster}─────────────────► tools
  tools ──Artifacts::path_for─────────────────────► the session's state (api side)

  loop, request ──Writer::push / own SQL on a handed-in connection──► storage
  tools ──nothing───────────────────────────────────────────────────►  (no database at all)
  everyone ──clock, log, temp directory, environment──► platform
  everyone ──values, never files──► config
```

- **Who calls this domain**: the api, and the config layer (for `PROTOCOL`, a string). Nothing else
  under `server/`.
- **The compaction is driven from the api**: it gathers the transcript, plans the cut, and calls this
  domain's `compact::compact` (with `transport::send` and `inbound::decode` as the round trip)
  itself. There is no request-side entry point for a compaction — the api assembles the pieces.
- **Who this domain calls**: storage (the writer, and reads on the connection it is handed), config
  (values), platform (the clock, the log, the temp directory). It never reaches the api.
- **What it does not know**: a front end, the command line, the api's doors, and any table beyond
  the column lists `storage::decode` hands out.
- **Threads**: one round thread per round (started by the api), and — when a provider connection
  carries a subprocess — that subprocess. The loop itself starts no thread.

## Inside one round

```text
  loop::run ──► env::build ──► Env ──► tools::execute (one call at a time)
   │                            ▲            │
   │                            └── ToolOutput { text, memo, spill }
   ▼
  request::start ──► assembly ──► transport ──► the provider
   │  ◄── inbound::decode (streamed, increments go to the sink)
   ▼
  the cut (new_blocks) ─┐
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
2. `request::hand_results` / `new_blocks` and `storage::shape`'s block columns — the cut decides what
   fills them, `Message::to_block` is the only conversion.
3. `tools::Spill` and `storage::Spill` — two shapes for one thing (the full text of an oversized
   result); `hand_results` maps them, field for field.
4. `config`'s `OutputLimits` and `Env.limits` — the `read` tool and the spill gate read the same
   numbers.
5. `transport::client::PROTOCOL` and the config's model entries: a provider's protocol name is one
   word in both.
6. `history::to_wire`'s grouping and `Message::ToolRequest`'s `first` / `text` — replay has to rebuild
   the message the model actually sent, not N separate calls.
7. The citation: storage spells it (`base36`), and both readers here parse it — `bash`'s
   `rewrite_refs` and `context`'s anchor lookup.
8. `tools::spec`'s parameter tables and `schema_of` — the manual and the argument filling are two
   readers of one declaration.

## Not wired yet

1. `config.server.request.streaming` has no reader **inside this domain**: the delivery mode belongs
   to the channel the pieces travel over, and `src/ipc/stream.rs` reads it.
2. The web tools' four limits are still constants (第 14 条).
3. The placeholder scheme from `memo.md` (the block stores a *name*, the words live in
   `config.tui.placeholder.*`) is not implemented: the built-in empty and interrupted texts are
   hard-coded and written into the block.
4. Two things this domain is written against but does not own are still open: the fork mode of a
   rewind (a `todo!()` on the api side), and the session write lock (`memo.md`).
