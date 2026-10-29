# api ARCHITECTURE

## Files

| Path | Holds |
|---|---|
| `mod.rs` | Module table and the public face; no functions. |
| `api.rs` | `Api`, `Conn`, `Refuse`; connection registration, hello/bye, `receive`, `idle_for`. |
| `route.rs` | The name tables and the JSON argument readers. |
| `relay.rs` | The connection table, the per-session buffer, forwarding, the writer's port, and the loop's event port. |
| `convert.rs` | Storage shapes into JSON. |
| `call/mod.rs` | Module table. |
| `call/store.rs` | Shared reads: the list, its rounds, their blocks, the session row. |
| `call/<name>.rs` | One read door each. |
| `post/mod.rs` | Module table. |
| `post/emit.rs` | The writer gate and the change event. |
| `post/<name>.rs` | One write door each. |
| `subscribe/mod.rs` | Module table. |
| `subscribe/<name>.rs` | One declaration each. |

## Relations

- `ipc::server` calls `Api::connect`, then `Api::receive` per inbound frame, then `Api::disconnect`.
  `receive` returns the reply to write, or nothing; every pushed frame goes through the `Sender`
  handed out by `connect`.
- `relay` reads two ports: the writer's pieces (content, through `Writer::relay`) and the loop's
  `Event`s (live signals); both become frames under the same delivery rules.
- `post` writes session data with `Api::writer` and asks `Api::register` for the loop-side effects
  (a state change, an interrupt, a drop).
- `subscribe` asks `Api::register` to open a turn and only changes the `relay` table.
- `call` never asks the loop or the relay for content; only the database.
- `relay` is the only writer to the connection queues and the only reader of the session buffers.

## Not wired

- Config-backed doors: `call.config`, `call.pools`, `post.default_model`, `post.reload`.
- The loop's `deliver` must return the round id.
- `ui/session` still names a contract module that no longer exists.
