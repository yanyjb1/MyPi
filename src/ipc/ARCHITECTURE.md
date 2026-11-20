# ipc ARCHITECTURE

## Files

| Path | Holds |
|---|---|
| `mod.rs` | Module table. |
| `line.rs` | Line codec: `encode`, `decode`, `Lines`, `MAX_LINE_BYTES`. Knows no frame contents. |
| `contract/message.rs` | `Frame { kind: String, name: String, payload: Value }`, reply and error constructors. |
| `contract/error.rs` | The error payload shape. |
| `contract/text.rs` | `Text`: one kind plus a machine key and optional arguments, or an upstream body. |
| `client.rs` | `Client`: connect + hello, `send`, `drain`, `alive`; a read thread feeds one channel. |
| `server.rs` | `claim`, `bind`, `serve`, `run`; one accept loop; `handle` per connection. |
| `stream.rs` | `write`: one connection's outbound queue until it closes, then the connection is shut. |
| `endpoint.rs` | Endpoint discovery: socket path or pipe name. |
| `transport/` | Unix socket, Windows local-only pipe. |

## Relations

- `server.rs::handle` takes `Api::connect`, starts a writer thread on `stream::write`, then
  reads frames and hands each to `Api::receive`. A returned frame is pushed into the same
  queue through `Conn::send`; nothing else writes the connection. The connection's table entry
  is held by a guard, so it is released however the thread ends, panic included.
- The writer thread shuts the connection when its queue ends. That is what wakes a reader
  blocked on a peer that died without closing: the read returns end of file, the guard releases
  the entry, and `serve` can stop.
- `serve` stops when `Api::idle_for` reports no connection for a whole grace period. A server
  that has not seen its first front end yet waits `BOOT` for it, because the front end starts
  the server and reaches for its endpoint a moment later.
- `client.rs` is used by `ui::session`, which sends frames and drains the connection; replies
  and pushes both arrive through `drain`.
- `contract` imports no server, ui, or transport code.
- `line.rs` knows bytes, not frames; `Frame` knows fields, not meanings.
