# session — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | module declarations and the type re-exports | public |
| `session.rs` | `Session`, `Draft`, the lifecycle, every public call, `forward` | `Session` public |
| `client.rs` | `Client`, `Request`/`Call`/`Post`/`Subscribe`, `Arrival`, `Push`, frame building, frame classification | private |
| `ingest.rs` | `apply` — one arrival into the caches, the flags, the outbox | private |
| `state.rs` | `State`: attachment, `busy`, `new_born`, `refused`, the in-flight init, the watch, the outbox | private |
| `structure.rs` | `Turn`, `Structure`: the round index and its rewind trim | public |
| `content.rs` | `ContentCache`: landed rows, the budget, the eviction, `Direction` | `Direction` public |
| `event.rs` | the exit vocabulary and the payload decoders | public |

## What it talks to

- **`ipc::client`**: one connection, `send` and `drain`. The session builds every frame itself.
- **`ipc::contract`**: `Frame` and `ErrorCode` only.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, drawing, themes, layout, keys, scrolling, rendered rows.

## Inside

```text
Session
 ├── client::Client      ── one connection, the sent queue, the answers still owed
 ├── state::State        ── attachment, flags, the in-flight init, the watch, the outbox
 ├── structure::Structure── the rounds of the attached session
 ├── content::ContentCache── the landed rows under a budget
 └── Draft               ── the makings of a session whose row does not exist yet
```

- `Session::forward` is the only exit. It pumps the connection and hands the queue over.
- `client::Client::receive` classifies a frame by `kind`: `stream` frames become `Push` values,
  every other frame is the answer to the oldest request still owed. `watch` and `bye` expect no
  answer; the queue drops them as later replies pass them by.
- `ingest::apply` is the only writer of `State`, `Structure` and `ContentCache` from outside.
- `Session::call_*`, `post_*`, `subscribe_*` are the only places a front end intent becomes a
  door call; `client::Request::frame` is the only place a frame is built.

## Must stay in step

1. `client::Call`/`Post`/`Subscribe` and the api's door table: one variant per door.
2. `client::decode_push` and api's stream payloads: a new piece kind needs an arm here and in
   `event::Piece`.
3. `ingest::apply` and the exit: a new event needs a landing site and a re-export.
4. `ingest` and the session id: every request that names a session is filtered against the
   attachment, or its answer is dropped as somebody else's.
5. `structure::Turn` and the wire's `ranges`: pairwise, closed intervals, id order.
6. `content::ContentCache` and `config.ui.session.preloadCache`: the budget is asked once at
   connect.
7. `Session::attach`/`detach` and both caches: both live exactly as long as one attachment.

## Not wired yet

1. `ui::tui` still reads `Session::view`, `Switches`, and the older call names.
2. `main` still calls the four-argument `Session::connect` and names `ipc::contract::Viewer`.
3. `src/lib.rs` still allows `ui::tui` to reach `ipc::client`; the session owns the connection, so
   that allowance shrinks once the TUI moves.
