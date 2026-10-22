# session

The layer between a front end and the api: one connection, one session's state, the block caches,
and a borrowed view.

## 1. Design

### One viewer, one session

A `Viewer` names one front end. A `Session` holds one viewer and watches one stored session at a
time; entering another leaves the first. Several viewers may watch the same session.

### Files

| file | holds |
|---|---|
| `mod.rs` | `Session`, `View`, `Switches` |
| `client.rs` | the connection, request queue, watch stream |
| `ingest.rs` | arrivals → state, window, structure, range |
| `state.rs` | session state, notices, usage |
| `window.rs` | content cache: landed window + unlanded tail |
| `shape.rs` | structure cache: blocks per round |
| `range.rs` | raw block interval for the tree |
| `command.rs` | command text → call |

### Entry points

```text
connect(cost, path, cwd, switches, viewer)  viewer()  bye()
init(session)  jump(turn)  fetch(num, older)  settled()  tree_range(start, end)
submit(text)  interrupt()  compact(...)  rewind(turn, block)
attach(id)  detach()  set_draft(...)  open_new()
rename(...)  set_cwd(...)  switch_model(...)  set_default_model(...)
set_profile(...)  delete(id)  list_sessions(...)  usage(...)  todo()  pools()
view()  text_of(block)  set_preload(n)  set_interrupted_text(text)  notice(...)
```

Changes land locally first and join one queue; `pump` sends the queue once per frame. Reads are
questions; their answers arrive through `ingest`.

### Opening

```text
enter(id)
  attach; clear structure and range
  Watch(id)
  Call::Init { session, limit: asked }
  Call::Meta { session }
  Call::Todo { session }
```

`Reply::Init` replaces the structure and the content window. `init` is the History landing: it
ignores the row this front end just created (`new_born`) and never sends a second `Init` while one
is in flight.

### Reads

| Call | Reply the session reads | Use |
|---|---|---|
| `Init { session, limit }` | `Init { turns, blocks, more }` | open |
| `Round { session, turn }` | `Round { turn, blocks, older, newer }` | jump |
| `Range { session, start, end }` | `Range(Vec<Block>)` | tree |
| `Older` / `Newer` | `Blocks { blocks, more }` | one side of the window |
| `Turn { session, turn }` | `Turn { turn, blocks }` | structure after a round |

`TurnShape` is `{ turn, blocks }`; each `TurnBlock` is `{ id, hash }`. `Reply::Init` carries the
structure newest first. `Reply::Round` replaces the content window and sets both latches from
`older` / `newer`. `Reply::Range` fills `RangeCache`, not the content window.

`jump` records a shrinking budget (32 → 8 → 4, floor 4); `settled` restores it. `fetch` asks for
`asked` blocks and names the edge block's round.

### Caches

| cache | holds | budget |
|---|---|---|
| content (`window.rs`) | landed blocks + tail | `Switches.preload`, at least 8 |
| structure (`shape.rs`) | per round: `id` + `hash`, newest first | unbounded |
| range (`range.rs`) | raw blocks for one interval | one interval |

`Window` trims on a landed append or a claimed tail block, only while no round is active; the oldest
`written` goes first. `Shapes` is replaced by `Reply::Init` and grows by `Reply::Turn`. `RangeCache`
has no contiguity rule and is independent of the content window.

### View

`View` borrows the attached session, flags, name, model, cwd, notices, todo, picker rows, pools,
usage, landed blocks, tail, optimistic interrupt row, structure, range, and generation.

The status line's `π` / `Tel` signal belongs to its own `Activity`; it does not pass through here.

### Invariants

1. A component never holds a connection; every api call is a `Session` method.
2. A change lands locally before it is sent.
3. Blocks are addressed by id, never by position.
4. `Window` trims by admission order, never during a round.
5. `claim` matches `(id, hash)`; no hash-only fallback.
6. The content hash has one recipe (`ContentKey`).
7. `init` does not re-read a row this front end just created.

## 2. Intent

- **One entry point.** Front ends do not invent a second way to ask.
- **One queue.** Intents go out in order; order is semantics.
- **Borrowed view.** Drawing does not copy the transcript.
- **Two caches.** A block can belong to two rounds, so structure and content stay separate.
- **Named reads.** Open returns structure + content; jump returns one round; tree returns a range.

## 3. Unresolved

1. The api lacks `Call::Init` / `Round` / `Range`, `Reply::Init` / `Round` / `Range`, and
   `TurnShape`; the session side calls them.
2. `Call::Round` has no limit field; the jump budget is local.
3. `Viewer` is still `i64`; the App's `Uuid` is not wired here.
4. `State::set_ctx_limit` has no caller, so `View.ctx_limit` is always `None`.
5. `Switches.asked` has no config field yet.
6. Tree fork rendering and `rewind_non_prune` are not implemented.

## 4. Config

| key | use |
|---|---|
| `config.ui.session.preloadCache` | content budget |
| `config.ui.session.askedEachTime` | blocks per read |
| `config.ui.placeholder.interrupted` | optimistic interrupt text |
| `model.contextWindow` | context gauge denominator |
