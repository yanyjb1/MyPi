# ipc

The wire between the server and a front end. Three parts: the contract both sides name, the
front end's client, and the server's adapter. Nothing else crosses.

## 1. Design

### Contract

`contract/` is the only shared vocabulary. It owns `Ask`, `Answer`, `Call`, `Reply`, `Post`,
`Subscribe`, `Event`, the block and session shapes, and the todo shapes. It depends on `serde` and
the command candidate type, and on neither `server` nor `ui`.

`ErrorCode` is structured; `Answer::Error` carries a code and a message. `Hello` carries
`PROTOCOL_VERSION`; a mismatch is refused before registration.

### Client

`client::Client` is the front end's only connection. It takes an `Endpoint`, registers with
`Hello`, and offers `call`, `post`, `submit`, `compact`, `watch`, and `bye`. A watch is a second
connection: `Hello`, then `Subscribe::Watch`, then the server only writes.

The client never names `server`.

### Server

`server` owns the database lock, the listener, the peer count, and one thread per connection. It
calls `server::api::Api` and nothing below it. `stream::pump` writes one watch's events and posts
`SessionWrite::Unwatch` when the stream ends.

### Transport and endpoint

`transport/` owns the platform mechanics:

- `unix.rs`: Linux and macOS, Unix sockets.
- `windows.rs`: Windows named pipes; interface present, implementation returns `Unsupported`.

`endpoint::Endpoint` is either `Unix { path }` or `Pipe { name }`. `Endpoint::file_path(db)` names
the JSON file a front end reads; `Endpoint::for_db(db, kind)` names the address the server binds.

Linux uses `$XDG_RUNTIME_DIR`, else `$TMPDIR/mypi/`. macOS uses `$TMPDIR/mypi/`. Windows uses
`%LOCALAPPDATA%\mypi\`, else `%TEMP%\mypi\`. Unix sockets get `0600`; Windows endpoint files live
under the user's directory.

### One database, one endpoint

The database lock decides who serves, not the endpoint. The server writes the endpoint file after
binding and removes it when it stops. A front end with no endpoint file has no server; it does not
guess, and it does not read `config`.

### Memory

No durable state in IPC. The server keeps the lock, listener, endpoint, and peer count. A client
keeps its command connection and, per watch, a channel and reader thread.

## 2. Invariants

1. One line is one JSON value; newline is the frame.
2. `Hello` before registration; a peer that never says it does not keep the server alive.
3. Protocol version mismatch is refused.
4. A watch connection is write-only after `Watch`.
5. The server posts `Unwatch` when a watch ends.
6. `buffered` filtering belongs to the backend API, not IPC.
7. `contract` names neither end.
8. `client` never names `server`.
9. `server` names only `server::api::Api`, not config, storage, or agent.

## 3. Open questions

1. Nothing reconnects after the server dies.
2. Windows named pipes are not written.
3. No authentication or encryption; local only.
4. The endpoint file is only removed on a clean stop.

## 4. Config

IPC reads no config.

- `config.server.platform.ipc` is read by the backend at startup to choose `Kind`; the choice is
  written into the endpoint file.
- `config.server.request.streaming` is read by the backend API; IPC does not filter.
