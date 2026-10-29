# api

The backend's one surface for front ends: a router on `(kind, name)`, and the session streams.

## Schema

One entry point:

```text
Api::receive(viewer, frame) -> Response
Response = Frame(reply) | Stream(receiver) | Silence
```

Name table:

| kind | names | reply |
|---|---|---|
| `register` | `hello`, `bye` | none (`Silence`); identity comes from the hello payload |
| `call` | `init` `blocks` `meta` `usage` `todo` `memo` `children` `config` `pools` `sessions` | each name's query result |
| `post` | `init` `name` `cwd` `switch` `profile` `todo_on` `todo_off` `delete` `interrupt` `rewind` `unwatch` `default_model` `reload` | `init` → `{session}`, `cwd` → `{cwd}`, `rewind` → `{cut}`, else `{}` |
| `subscribe` | `watch` `submit` `compact` | `watch` → the stream, `submit`/`compact` → `{turn}` |
| `notice` | outbound only | inbound `notice` is `unknown_name` |

A reply's `kind` and `name` echo the request. Errors are payloads: `unknown_name`,
`bad_payload`, `busy`, `internal`. `busy` means the session is in a round, or a reload has
sessions running.

Streams belong to sessions, not viewers:

```text
streams: HashMap<i64, SessionStream>
SessionStream { turn: Option<TurnBuffer>, subscribers: Vec<Subscriber> }
Subscriber { viewer, role: Writer | Watcher, turn: Option<i64>, tx: Sender<Frame> }
```

- One stream per session, many subscribers on it. A viewer attaches through `watch`
  (Watcher, whole stream), `submit` or `compact` (Writer, one turn).
- The turn buffer holds one round's frames; a new subscriber first gets the buffered frames
  it is entitled to, then increments.
- A Watcher receives everything on its stream. A Writer skips the post-metadata frames it
  caused itself (`meta`, `rewound`, `deleted`).
- `notice` goes to every subscriber of the session's stream regardless of role or watch
  state; with nobody on it, it is dropped.
- `round_ended` clears the turn buffer and drops turn filters pointing at it; `deleted`
  destroys the whole stream. A failed channel send removes the subscriber on the spot.

## Design intent

- One router: the name table is data in `route.rs`, so a new operation is one file plus one
  line, and no other layer ever inspects a name.
- Streams belong to sessions: several front ends on one session share one fan-out point, and
  the turn buffer replays late joiners without a history store.
- Roles are declarations, not state the server assigns: what a viewer receives follows from
  which request it sent, so there is no role bookkeeping to corrupt.
- `Api` methods are `&self`: all mutable state sits in the driver's one global lock, so the
  ipc threads need no per-door synchronization.

## Unresolved

- A viewer that never sends `unwatch` keeps its channel until a write fails. There is no
  liveness probe; a dead connection is noticed only when a frame is sent.

## Config fields

None directly. The api passes `Config` through to `storage`, `agent`, and `config`; it reads
no keys of its own.
