# ipc ARCHITECTURE

## Files

| Path | Holds |
|---|---|
| `mod.rs` | Module table. |
| `line.rs` | Line codec: `encode`, `decode`, `Lines`, `MAX_LINE_BYTES`. Knows no frame contents. |
| `contract/message.rs` | `Frame { kind: String, name: String, payload: Value }`, reply and error constructors. |
| `contract/error.rs` | `ErrorCode`, the error payload shape. |
| `client.rs` | `Client`: connect + hello, `ask`, `send`, `drain`; a read thread feeds one channel. |
| `server.rs` | `claim`, `bind`, `serve`, `run`; one accept loop; `handle` per connection. |
| `stream.rs` | `write`: one connection's outbound queue until it closes. |
| `endpoint.rs` | Socket path discovery. |
| `transport/` | Unix socket, Windows pipe. |

## Relations

- `server.rs::handle` takes `Api::connect`, starts a writer thread on `stream::write`, then
  reads frames and hands each to `Api::receive`. A returned frame is pushed into the same
  queue through `Conn::send`; nothing else writes the socket. On exit it calls
  `Api::disconnect` and drops the handle, which closes the queue.
- `serve` stops when `Api::idle_for` reports no connection for a whole grace period.
- `client.rs` is used by `ui::session`, which sends frames and drains the connection. `ask`
  remains for a caller that waits on one answer; stream frames carry another kind and never
  match it.
- `contract` imports no server, ui, or transport code.
- `line.rs` knows bytes, not frames; `Frame` knows fields, not meanings.

## Not yet wired

- `main.rs` still builds `Api::new(cfg)`; it must open the writer first and hand `Api::new` the
  database path, the writer, the loop's register, and the relay.
