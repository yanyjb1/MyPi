# ipc architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`. This file
describes the code as it stands, not how it got here.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the protocol's own vocabulary (`Ask`, `Answer`), the endpoint's path |
| `frame.rs` | the framing: `encode`, `decode`, `MAX_LINE_BYTES`, and `Lines` (one connection's read face, with the scanner behind it) |
| `server.rs` | the server half: the database lock (`Claim`, `claim`), the bound endpoint (`Bound`, `bind`), `serve`, `run`, the accept loop, one connection's handler, the register (`Peers`), replacing a leftover endpoint, the endpoint's permissions |
| `stream.rs` | the server's half of one watch: a session's stream becomes lines (`pump`); `pub(crate)` |
| `client.rs` | the front end's half: `Client` (connect, ask, change, open a stretch, watch, leave) and the reader behind a watch |

## The public surface

| item | what it does |
|---|---|
| `Ask`, `Answer` | what crosses the wire, in each direction |
| `socket_path(db)` | where this database's server listens |
| `server::claim(db)` | try to serve this database; `None` means somebody already does |
| `server::bind(cfg, db, claim)` | create the endpoint, after the lock |
| `server::serve(bound)` | answer until no front end is left |
| `server::run(cfg, db, claim)` | `bind` then `serve`, for `mypi --server` |
| `client::Client::connect(cost, path, viewer)` | connect and register |
| `Client::call`, `::post`, `::submit`, `::compact`, `::bye` | the command connection's actions |
| `Client::watch(session)` | a connection of its own, and the channel its items arrive on |

## Relationships

```text
   a front end process
        │
        │  Client: one command connection, plus one per watched session
        ▼
   ┌────────────┐   one endpoint   ┌──────────────────┐
   │ ipc::client│◄────────────────►│  ipc::server     │
   │ frame::Lines│                 │  accept loop     │
   └────────────┘                  │  a thread per    │
                                   │  connection      │
                                   └────────┬─────────┘
                                            │ the api's three doors
                     ┌──────────────────────┼───────────────────────┐
                     ▼                      ▼                       ▼
            api::call::answer      api::post::apply      api::subscribe::{watch,
                     │                      │             submit, compact}
                     └──────────┬───────────┴───────────────────────┘
                                ▼
                    storage and the agent (below the api)

   a stream: api → stream::pump → the watch's own connection → the front end
```

- **Who calls ipc**: `main.rs` (which role to play, and the server's own path) and a front end
  (through `Client`). Nothing else.
- **Who ipc calls**: the api's three doors, and `post::apply` with `Unwatch` when a watch ends;
  `platform::ipc` for the endpoint, `platform::env` for the runtime directory, `platform::log` for
  its own messages; `config` for two values.
- **Who reads the database**: nobody here. This layer opens the database file only to take its lock.
- **Who ipc does not know**: `storage`, `agent`, a front end, the command line, the tools, and the
  shape of a request body. The guards in `lib.rs` keep the two sides apart: nothing under `server/`
  may name `ipc`, and nothing here may name a front end.

## Threads

| thread | created by | lives |
|---|---|---|
| the accept loop | `serve` | until the server stops |
| one per connection | the accept loop | one connection's life: it answers lines, or becomes the watch's writer and stays until that stream ends |
| a watch's reader | `Client::watch`, on the front end's side | until the receiving end is dropped, or the connection ends |

## Parts that must stay in step

1. `Ask` and `Answer` and the api's shapes they carry — a new api shape crosses only if these enums
   name it.
2. `socket_path` and both ends: the server binds the path the front end connects to, and both come
   from the same function.
3. `frame::MAX_LINE_BYTES` and the cap in the read face.
4. `Hello` (the one thing that registers a peer) and the grace period the server waits before it
   stops.
5. `Client::watch`'s opening move and the server's handling of a watch: the connection turns into a
   stream at that point, and only there.

## Not wired yet

1. The front end: `main.rs` connects and then stops with a message. Its connect calls still expect a
   front-end-id factory that does not exist; the caller must make one `Viewer` above this layer and
   pass it to `Client::connect`.
2. The Windows named pipe: the platform layer's Windows bodies are `unimplemented!()`.
3. Nothing reconnects: a front end whose server dies makes no second attempt.
4. One-shot commands do not use this layer (`mypi --sessions` reads the database directly).
