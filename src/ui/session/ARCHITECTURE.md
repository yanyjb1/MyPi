# session — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds | to the outside |
|---|---|---|
| `mod.rs` | module declarations; re-exports `Arrival`, `Content`, `Direction`, `Packet`, `Session`, `Switches`, `Structure`, `Turn` | public |
| `session.rs` | `Session` (the entry point), `Switches`, `Draft`, the lifecycle, every public call | `Session`, `Switches` public |
| `client.rs` | `Client` (connection, queue, watch), `Request`, `Arrival`, `Round`, `Kind` | `Arrival` public; the rest private |
| `ingest.rs` | `apply` — one arrival into structure, content, flags, outbox | private |
| `state.rs` | `State` (attachment, `busy`, `new_born`, `refused`, `outbox`) | private |
| `structure.rs` | `Structure` (rounds), `Turn` (a round's intervals) | public, reached through `Session::local_struct_snap` |
| `content.rs` | `ContentCache` (the archive), `Content`, `Packet`, `Boundary`, `ToolOutput`, `Direction` | `Content`, `Packet`, `Direction` public; the cache itself private |

## What it talks to

- **`ipc::client`**: connect, one call and its reply, one post and its outcome, one subscription
  (`Watch`, `Submit`, `Compact`), bye. The watch arrives on a channel of its own.
- **`ipc::contract`**: `Call`, `Post`, `Subscribe`, `Reply`, `PostDone`, `Opened`, `Event`,
  `Block`, `NewBlock`, `BlockKind`, `TurnShape`, the ask and write groups, `Viewer`, `Retention`.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, drawing, themes, layout, keys, scrolling, notice lifetimes,
  rendered rows, the picker's page, the tree's page.

## Inside

```text
Session
 ├── client::Client      ── one command connection, one watch stream, one queue
 ├── state::State        ── attachment, flags, outbox
 ├── structure::Structure── the rounds of the attached session
 ├── content::ContentCache── landed blocks by id, streamed blocks by boundary
 └── Draft               ── the makings of a session whose row does not exist yet
```

- `Session::forward` is the only exit. It pumps the connection, hands every queued `Arrival` over,
  and empties the queue. It is also the only place the upstream loop runs.
- `ingest::apply` is the only writer of `State`, `Structure` and `ContentCache` from outside. It
  drops arrivals that do not belong to the attachment, then queues the rest.
- `Session::local_struct_snap` is the only reader path into the structure.
- `Session::call_*`, `post_*`, `subscribe_*` are the only places a front end intent becomes a door
  call; `client::Client::send` is the only place the wire is touched.

## Must stay in step

1. `client::Request` and the doors: one variant per door shape (`Call`, `Post`, `Subscribe`,
   unwatch, bye). A new door has to be added here and in `Session`.
2. `ingest::apply` and the contract: a new `Event`, `ContentEvent`, `SessionEvent` or `Reply`
   variant has to be routed here.
3. `ingest::belongs` and the session id: every door call that names a session has to answer with
   it, or its reply is dropped as somebody else's.
4. `content::Boundary` and the stream: the identity used to merge pieces must be the identity the
   server sends, block kind plus tool call id.
5. `content::merge_landed` and the landed order: the n-th streamed block binds the n-th new id, so
   both sides must keep arrival order.
6. `structure::Turn` and the landed blocks: a landed batch opens or extends an interval; a round
   that never landed has no interval and cannot be read.
7. `Session::attach`/`detach` and the caches: both caches live exactly as long as one attachment.

## Not wired yet

1. `ui::tui` still reads `Session::view`, `init`, `jump`, `fetch`, `settled`, `tree_range`,
   `notice`, `text_of`, `set_draft`, and the old `Switches` fields.
2. `main::front_end_with` still passes a `Viewer` to `Session::connect`, which now generates one.
3. `src/lib.rs` still allows `ui::tui` to reach `ipc::contract` and `ipc::client`; the session owns
   the connection, so that allowance has to shrink.
