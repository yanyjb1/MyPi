# session — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds | to the outside |
|---|---|---|
| `mod.rs` | `Session` (the entry point), `View` (the borrowed read model), `Switches` | public |
| `client.rs` | `Client` (the connection, the queue, the stream), `Request`, `Arrival`, `Round`, `Kind` | private |
| `ingest.rs` | `apply` — one arrival → state, window, structure and range; `Follow` | private |
| `state.rs` | `State`, `Notice`, `Level`, `Life`, `UsagePanel` | types public; `State` reached through `View` |
| `window.rs` | `Window` (the content cache), `WindowBlock`, `LiveBlock`, `LiveTail`, `BlockRef`, `ToolOutput` | `BlockRef`, `LiveBlock`, `LiveTail`, `ToolOutput` public; the rest through `View` |
| `shape.rs` | `Shapes` (the structure cache), `Shape` | public; written by `ingest`, read through `View` |
| `range.rs` | `RangeCache` (raw blocks for the tree) | public through `View` |
| `command.rs` | `Session::command` — a `CommandMeta` and its arguments → the call | public through `Session` |

## What it talks to

- **The protocol layer's client**: connect, ask, change, submit, compact, watch, bye.
- **The api's shapes**: the four doors and what they answer, the viewport's placed block, the round
  structure, the session row, the meta answer, usage rows, retention, model id, usage wording,
  candidate pools, session meta, block, unlanded block, todo phase, endpoint cost.
- **The configuration seam**: `Switches` carries the values a front end read at startup.
- **Knows nothing about**: terminals, drawing, themes, layout, keys, scrolling, folding, anchors,
  grouping blocks into cards, caches of rendered rows, the picker's page, the tree's page.

## Inside

```text
Session
 ├── client::Client   ── one command connection, one stream per watched session
 ├── state::State     ── what the front end knows about the session
 ├── window::Window   ── the content cache
 ├── shape::Shapes    ── the structure cache
 └── range::RangeCache── the tree's raw block interval
```

- `Session::pump` is the only pump: it takes what the stream pushed, sends the queue in order, and
  hands every arrival to `ingest::apply`. It clears `init_in_flight` when the matching `Call::Init`
  is answered or fails.
- `ingest::apply` is the only writer of `State`, `Window`, `Shapes` and `RangeCache` from outside.
  It answers with a `Follow`, which `Session` turns into requests.
- `Session::view` is the only reader path: it borrows `State`, `Window`, `Shapes` and `RangeCache`
  into `View`.
- `Session::command` is the only place a command's text becomes a parsed value.

## Must stay in step

1. `client::Request` and the api's doors: one variant per door call, plus watch, unwatch and bye.
2. `ingest::apply` and the api's reply and stream types: a new variant on either side has to be
   routed here.
3. `View`'s fields and the state, window, structure and range: the view borrows them, so a new field
   means four edits.
4. `window::BlockRef` and the fields of stored and unlanded blocks: its accessors list every field a
   render side may read.
5. The content hash: `storage::shape::ContentKey` is the one recipe — the api hashes with
   `Block::content_key()`, this layer with `BlockRef::content_key()`.
6. `Window` admission order (`written`) and `trim`: a new admission path must use `admit_placed` or
   `admit_block` or the eviction order stops matching the design.

## Not wired yet

1. `lib.rs` does not declare `ui`, and `Cargo.toml` has no `ratatui` / `crossterm`: nothing under
   `src/ui` is part of the crate's build.
2. `src/ui/tui` is the terminal front end. Its history, input, reserved and render paths do not read
   a `View` yet, and its frame loop does not exist yet.
3. The api has no `Call::Init` / `Reply::Init`, `Call::Round` / `Reply::Round`,
   `Call::Range` / `Reply::Range` or `TurnShape`; the session side names them.
4. `Viewer` is still `i64`; the App's `Uuid` and the shared `src/id.rs` are absent.
5. `State::set_ctx_limit` has no caller, so `View.ctx_limit` is always `None`.
6. `Session::init`, `jump`, `fetch`, `settled` and `tree_range` have no caller in the front end.
7. Tree fork rendering and `rewind_non_prune` are not implemented.
