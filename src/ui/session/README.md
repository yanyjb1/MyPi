# session

The layer every front end sits on: one IPC connection, the two caches of the session it follows,
and one exit through which everything the connection produced reaches the front end.

## 1. Design / Schema

### One connection per front end

Each front end instance owns one `Session`. It registers under an id it generates itself and is
attached to at most one stored session at a time. `hello` opens the connection, `bye` closes it.
Several front ends may follow the same stored session.

### The exit

One queue of `Event`, in arrival order. `Session::forward` pumps the connection and hands the
queue over.

| group | events |
|---|---|
| pushed | `Live(Piece)`, `Notice`, `Structure`, `Change` |
| answered | `Opened`, `Init`, `Blocks`, `Meta`, `Usage`, `Todo`, `Memo`, `Children`, `Config`, `Pools`, `Sessions` |
| write outcomes | `Created`, `Done`, `Busy`, `Cut`, `DefaultModel` |
| failure | `Failed { kind, message }` |

Every pushed frame names the session it belongs to; a push for another session is dropped. The
events carry the upstream payload: `Piece` is the stream's own shape (`text`, `thinking`,
`arguments`, `boundary`, `tool_start`, `tool_progress`, `tool_end`), `Block` is one `blocks` row,
`Change` is one write another front end made.

### The caches

```text
Structure  turns: Vec<Turn>              newest first
Turn       { id, blocks: Vec<Range> }    the closed block id intervals of one round
Content    blocks: BTreeMap<i64, Row>    landed rows, each with an admission time
```

- `Structure` is the index: which rounds exist and which blocks they own. `local_struct_snap`
  hands out a frozen copy. `Init` replaces it, a round-structure push inserts one round, a
  rewind trims it from the cut.
- `Content` is the raw text: `blocks` rows by id. Read through `landed()`.
- A row's admission time is a counter taken when it enters the cache; seeing the row again
  refreshes it.
- Eviction: while over budget, drop the earliest admission, then the lowest id. It runs after an
  `init` or `blocks` answer and only while the session is idle.
- Streamed pieces are not cached; they leave through `Live`.
- Both caches live exactly as long as one attachment.

### Entry points

```text
runtime     connect(endpoint, cwd)  forward()  bye()  set_draft(...)
lifecycle   attach(id)  detach()

reads       call_init(session)  call_turn(turn)  call_side(num, direction)
            call_meta()  call_usage()  call_todo()  call_memo(op, scope, name)
            call_children()  call_config(key)  call_pools()  call_sessions(under)

writes      post_init()  post_name(name)  post_cwd(path)  post_switch(model)
            post_delete(session)  post_interrupt()  post_rewind(turn, block)
            post_default_model(model)

rounds      subscribe_watch(session)  subscribe_submit(session, prefix, text, suffix)
            subscribe_compact(session, retention, focus)

local       attached()  busy()  refused()  local_struct_snap()  landed()
```

### Invariants

1. One `Session` per front end instance.
2. One attachment at a time; changing it is an explicit `detach` then `attach`.
3. The exit is the arrival order.
4. A pushed frame belongs to its session; nothing else is kept from it.
5. Structure is the only index; content holds landed rows only.
6. Eviction is time first, then id.
7. No payload is rewritten on the way out.

## 2. Design intent

- **One connection, one queue.** Intents leave in the order the front end made them; order is
  part of what a session means.
- **The front end parses, the session translates.** A door has one author, so a new front end
  cannot invent a second dialect for the same action.
- **Two caches.** Structure answers "which rounds exist and which blocks they own"; content
  answers "what the block says". A front end asks for what it draws; the raw material stays here.
- **Time before sequence.** The block that has been here longest is the one the reader has moved
  away from; ids only break ties.
- **The exit is the arrival.** A front end that asked for usage, todo, config, a session list or a
  memo gets the answer as an event; a front end that is watching gets each piece as it comes.

## 3. Unresolved

1. `ui::tui` and `main` still speak an older session surface; they are wired separately.
2. A rewind has no round-structure frame upstream; the structure is trimmed from the cut the
   front end received.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.session.preloadCache` | blocks kept in the content cache; asked once at connect |
