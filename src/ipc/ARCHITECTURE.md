# ipc architecture

What the pieces are and how they relate. The design is in `README.md`.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations only |
| `contract/` | the shared wire vocabulary: envelope, data, call, post, stream |
| `frame.rs` | `encode`, `decode`, `Lines`, `MAX_LINE_BYTES` |
| `transport/` | platform connection mechanics: `Kind`, `Listener`, `Stream`, `bind`, `connect`, `unlink` |
| `endpoint.rs` | `Kind`, `Endpoint`, endpoint file read/write/remove, fingerprint, runtime directory |
| `client.rs` | `Client`: connect, call, post, submit, compact, watch, bye |
| `server.rs` | `Claim`, `claim`, `Bound`, `bind`, `serve`, `run`, accept loop, one connection, `Peers` |
| `stream.rs` | `pump`: one watch's events to one connection |

## Public surface

| item | what it does |
|---|---|
| `contract::*` | wire types shared by both sides |
| `endpoint::Endpoint` | one database's address |
| `endpoint::{read, write, remove, file_path}` | endpoint discovery |
| `transport::{bind, connect, unlink}` | platform mechanics |
| `client::Client` | the front end's connection |
| `server::{claim, bind, serve, run}` | the server's side |

## Relationships

```text
ui::session -> ipc::client -> ipc::contract
                                   ^
                                   |
ipc::server -> server::api --------+
```

- `ui::session` names `ipc::client`, `ipc::contract`, and `ipc::endpoint`.
- `ipc::client` names `contract`, `frame`, `transport`, `endpoint`; never `server`.
- `ipc::server` names `contract`, `frame`, `transport`, `endpoint`, `stream`, and
  `server::api::Api`; never config, storage, or agent.
- `ipc::contract` names neither `server` nor `ui`.
- `ipc::transport` and `ipc::endpoint` name neither `server` nor `ui`.

## Threads

| thread | created by | lives |
|---|---|---|
| accept loop | `server::serve` | until the server stops |
| one per connection | accept loop | one connection; a watch becomes its writer |
| watch reader | `Client::watch` | until the channel is dropped or the stream ends |

## Must stay in step

1. `PROTOCOL_VERSION` and `Hello`.
2. `Endpoint` and both sides: the server binds what `Endpoint::for_db` names; the front end reads
   what `server::bind` wrote.
3. `frame::MAX_LINE_BYTES` and `Lines`.
4. `Hello` registration and `Peers` grace.
5. `Client::watch` opening move and `server`'s watch handling.
6. `contract::ContentKey` hash fields and both sides' id-claim.

## Not wired yet

1. Windows named pipes: the interface exists; calls return `Unsupported`.
2. No reconnect.
