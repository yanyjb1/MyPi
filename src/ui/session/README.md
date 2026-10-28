# session

The layer every front end sits on: one IPC connection, the caches of the session it is attached to,
and one exit through which everything the connection produced reaches the front end.

## 1. Design / Schema

### One session per front end

Each front end instance owns one `Session`. It registers with the server under an id it generates
itself, and it is attached to at most one stored session at a time. Several front ends may be
attached to the same stored session.

### Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and public type re-exports |
| `session.rs` | `Session`, `Switches`, the lifecycle, every public call, the exit |
| `client.rs` | the connection, the request queue, the watch, the upstream loop, `Arrival` |
| `ingest.rs` | the one inbound writer: arrival → structure, content, flags, outbox |
| `state.rs` | attachment, `busy`, `new_born`, `refused`, `outbox` |
| `structure.rs` | `Turn { id, blocks }`, `Structure { turns }`, snapshots |
| `content.rs` | `Content { kind, packet }`, landed by id, live by boundary, `Direction` |

### Lifecycle

```text
connect()          an empty session: a connection, no attachment
attach(id)         record the attachment; the watch is a separate subscription
detach()           drop the attachment, both caches, the flags, and the queued requests
switch             detach() then attach(new id)
created            detach() then attach(the new id)
forward()          pump the connection, hand the queue over, empty it
bye()              leave the server
```

`attach` only records. Watching a session is `subscribe_watch(id)`, so a front end that attaches
also decides when the stream starts.

### Structure cache

```text
Turn      { id: i64, blocks: Vec<RangeInclusive<i64>> }
Structure { turns: Vec<Turn> }
```

- One per attachment, append-only, newest round first.
- Block ids rise globally; when two sessions write at once a round owns several intervals, so a turn
  holds a list of closed intervals instead of one range.
- `call_init` writes the server's first frame. Later rounds are opened by landed blocks.
- `local_struct_snap` returns a frozen copy the caller owns; later updates go into the session's
  structure.
- `detach` drops the whole structure.

### Content cache

```text
Content { kind: BlockKind, packet: Packet }
Packet  = Live(NewBlock) | Landed(Block)

landed: id → Content
live:   boundary → Content
```

- `packet` is the upstream packet as it arrived; `kind` is its block kind. Consumers decode it.
- Streamed pieces merge into one entry by boundary (block kind plus tool call id), in arrival order.
- Landing binds the n-th streamed block to the n-th new id and moves it into `landed`.
- `landed` holds blocks by id for the life of the attachment. There is no budget and no eviction.
- A whole replacement (`init`, a detached frame) bumps `generation`; a reader's derived cache is
  invalid across it.

### Exit

```text
client upstream loop → ingest::apply → structure / content / flags / outbox
                                      → Session::forward → front end
```

- Everything that belongs to the attached session is queued as it arrived; arrivals from other
  sessions are dropped.
- The queue holds `Arrival`: stream events, answers with the call they answer, post outcomes,
  round openings, a watch ending, a failure with its kind.
- Order in the queue is arrival order. The session does not regroup, classify, or redraw them.

### Entry points

```text
runtime     connect(endpoint, cwd, switches)  forward()  bye()
lifecycle   attach(id)  detach()  apply_ui_config(switches)

reads       call_init(session, limit)  call_turn(turn)  call_side(num, direction)
            call_meta()  call_usage(lang)  call_todo()  call_memo(op, scope, name)
            call_children()  call_config(key)  call_pools()  call_sessions(under)

writes      post_init()  post_name(name)  post_cwd(path)  post_switch(id)
            post_profile(profile)  post_todo_on(call_id)  post_todo_off(call_id)
            post_delete(id)  post_interrupt()  post_rewind(turn, block)
            post_default_model(id)  post_reload()  post_memo(write)  post_unwatch()

rounds      subscribe_watch(session)  subscribe_submit(session, prefix, text, suffix)
            subscribe_compact(session, retention, focus)

local       local_struct_snap()  local_settle()
```

The front end parses its own arguments; each method takes plain values and translates them into one
upstream door call. `call_turn` and `call_side` compute the block intervals themselves from the
structure cache.

### Invariants

1. One `Session` per front end instance.
2. The front end parses arguments; the session translates them.
3. The session owns the connection, both caches, and the exit.
4. One attachment at a time; changing it is an explicit `detach` then `attach`.
5. The structure is append-only for the life of an attachment.
6. Content is keyed by id when landed and by boundary while streaming; landing binds positionally.
7. No payload is rewritten on the way out.
8. One exit, in arrival order.

## 2. Design intent

- **One connection, one queue.** Intents leave in the order the front end made them; order is part
  of what a session means.
- **The front end parses, the session translates.** A door call has one author, so a new front end
  cannot invent a second dialect for the same action.
- **Two caches.** Structure answers "which rounds exist and which blocks they own"; content answers
  "what the blocks say". A block can belong to two rounds and content outlives a round, so the two
  change at different times.
- **Intervals, not one range.** Block ids are global and writes interleave, so one round's ids are
  contiguous only by luck.
- **Positional landing.** While a block streams it has no id; it is matched by kind and tool call id.
  The id arrives with the landed batch, so the n-th streamed block takes the n-th new id.
- **The exit is the arrival.** A front end that asked for usage, todo, config, a session list, or a
  memo gets the same object the server sent, next to the call that asked. The session never grows a
  second vocabulary for facts it merely passes through.
- **Frozen structure.** A reader keeps the snapshot it was handed; the session keeps writing into
  its own structure.

## 3. Unresolved

1. Stale answers are dropped by attached session id only. A detach and re-attach of the same id can
   let an older answer land.
2. `call_side` shrinks its own budget after every turn read and restores it on `local_settle`; no
   caller can read the current size.
3. Blocks replies merge into the content cache by id, so a rewind cannot remove the blocks it cut.
   The next `call_init` frame replaces the cache whole.
4. Notice text for a failed request is carried by the failure arrival; how long it stands, and
   whether it is shown at all, belongs to the front end.
5. The content archive has no read path: a front end renders from the content it was handed, and
   only the structure has a snapshot accessor.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.session.askedEachTime` | blocks per side read; carried by `Switches.asked` and `apply_ui_config` |
