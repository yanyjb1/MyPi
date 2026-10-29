# ipc ARCHITECTURE

## Files

| Path | Holds |
|---|---|
| `mod.rs` | Module table. |
| `line.rs` | Line codec: `encode`, `decode`, `Scanner`, `Lines`, `MAX_LINE_BYTES`. Knows no frame contents. |
| `contract/` | The wire vocabulary; the only code both processes name. |
| `contract/message.rs` | `Frame { kind, name, payload }`, `Kind` (five values), reply and error constructors. |
| `contract/error.rs` | `ErrorCode` (four values), the error payload shape. |
| `contract/register.rs` | `Viewer`. |
| `contract/notice.rs` | `Notice`, `NoticeLevel`. |
| `contract/data.rs` | `Block`, `BlockKind`, `NewBlock`, `SessionMeta`, `Spill`, `TodoPhase`, `ContentKey`. Untouched by this wave. |
| `contract/call/` | One file per read name: request and reply payloads, no behavior. |
| `contract/post.rs` | The write names' payloads (`ask::`), reply payloads (`reply::`), `ModelId`, `Cut`, `SessionWrite`, `ServerWrite`, `MemoWrite`. |
| `contract/subscribe.rs` | The three request payloads, `Retention`, `StartKind`, `TurnUsage`, the eight stream frame payloads. |
| `client.rs` | `Client`: hello, `ask` (serial, mutex), `watch` (own connection, reader thread into a channel), `bye`. |
| `server.rs` | `claim`/`bind`/`serve`/`run`, per-connection `handle`, `Peers` counting. |
| `stream.rs` | `pump`: frames to one connection until either end gives up, then one `post.unwatch`. |
| `endpoint.rs` | Socket path discovery. Unchanged. |
| `transport/` | Unix socket, Windows pipe. Unchanged. |

## Relations

- `server.rs::handle` decodes a `Frame`, takes `viewer` from `register.hello`, and calls
  `api.receive(viewer, frame)`. Per `Response` it writes a frame, hands the connection to
  `stream::pump`, or stays silent. It reads no name, checks no kind semantics.
- `client.rs` is used by `ui::session` and `oneshot.rs`; both must migrate to `ask`/`watch`.
- `contract` never imports `server`, `ui`, or transport code. Both processes meet here.
- `line.rs` knows bytes, not frames; `Frame` knows fields, not meanings.

## Not yet wired

- `ui::session`, `ui::tui`, and `oneshot.rs` still name the old contract (`Call`, `Reply`,
  `Post`, `Event`, ...). That is the next wave's wiring work.
- `stream.rs` sends `post.unwatch` without a session when no frame carried one.
