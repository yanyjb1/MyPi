# session — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds | to the outside |
|---|---|---|
| `mod.rs` | `Session` (the entry point), `View` (the borrowed read model), `Switches` (the knobs a front end hands in) | public |
| `client.rs` | `Client` (the connection, the queue, the stream), `Request`, `Arrival`, `Round`, `Kind` | private to the layer |
| `ingest.rs` | `apply` — one arrival → state, window and structure; `Follow` | private to the layer |
| `state.rs` | `State`, `Notice`, `Level`, `Life`, `UsagePanel` | the types are public; `State` is reached through `View` |
| `window.rs` | `Window` (the content cache), `WindowBlock`, `LiveBlock`, `LiveTail`, `BlockRef`, `ToolOutput`, `Want` | `BlockRef`, `LiveBlock`, `LiveTail` and `ToolOutput` are public; the rest is reached through `View` |
| `shape.rs` | `Shapes` (the structure cache), `Shape` | public; written by `ingest`, read through `View` |
| `command.rs` | `Session::command` — a `CommandMeta` and its arguments → the call | public, through `Session` |

`README.md` is the design; this file is the wiring.

## What it talks to

- **Calls**: the protocol layer's client (connect, ask, change, submit, compact, watch, bye), its
  socket path and viewer id, and the command table (a `CommandMeta` and its arguments, in
  `command.rs`).
- **Names the api's shapes**: the four doors and what they answer, the viewport answer's placed
  block, the round structure answer, the session row, the meta answer, the usage rows, the retention
  of a compaction, the model id, the usage wording, the candidate pools, the session meta, the block,
  the unlanded block, the todo phase, and the endpoint cost.
- **Knows nothing about**: terminals, drawing, themes, layout, keys, scrolling, folding, anchors,
  grouping blocks into cards, caches of rendered rows, the picker's page, the tree's page. No front
  end's drawing decision lives here.
- **Is called by**: a front end's frame loop. Today that is the terminal front end under
  `src/ui/tui`, which is not part of the build; a web front end would call the same methods.

## Inside

```text
Session
 ├── client::Client   ── one command connection, one stream per watched session
 ├── state::State     ── what the front end knows about the session
 ├── window::Window   ── the content cache
 └── shape::Shapes    ── the structure cache
```

- `Session::pump` is the only pump: it takes what the stream pushed, sends the queue in order, and
  hands every arrival to `ingest::apply`.
- `ingest::apply` is the only writer of `State`, `Window` and `Shapes` from outside. It answers with
  a `Follow`, which `Session` turns into requests.
- `Session::view` is the only reader path: it borrows `State`, `Window` and `Shapes` into `View`.
- `Session::command` is the only place a command's text becomes a parsed value; a surface keeps its
  own commands to itself.

## Must stay in step

1. `client::Request` and the api's doors: one variant per door call, plus watch, unwatch and bye.
2. `ingest::apply` and the api's reply and stream types: a new variant on either side is a compile
   error on the other, and a new shape has to be routed here or it is dropped in silence.
3. `View`'s fields and the state, the window and the structure: the view borrows them, so a new field
   means four edits.
4. `window::BlockRef` and the fields of the stored and unlanded blocks: its accessors list every
   field a render side may read.
5. The content hash: `storage::shape::ContentKey` is the one recipe — the api hashes with
   `Block::content_key()`, this layer with `BlockRef::content_key()`.

## Not wired yet

1. `lib.rs` does not declare `ui`, and `Cargo.toml` has no `ratatui` / `crossterm`: nothing under
   `src/ui` is part of the crate's build.
2. `src/ui/tui` is the terminal front end. Its history, input, reserved and render paths do not read
   a `View` yet, and its frame loop does not exist yet.
3. The tree, the round list, replay and log reads have no api to call (`README.md` §3).
