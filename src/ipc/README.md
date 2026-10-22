# ipc

The wire between the server and a front end: one line, one value, the api's shapes in both
directions.

## 1. Design

### Two halves

| half | holds | does |
|---|---|---|
| server | database lock, endpoint, register | accepts, answers, streams, stops when no front end is left |
| front end | one command connection, one per watched session | asks, changes, watches, leaves |

`mypi --server` is the server alone. Plain `mypi` looks for one; if none, it takes the lock, binds
the endpoint, starts the server, and spawns the front end.

### Wire

Only three lines are the protocol's own:

| line | direction | means |
|---|---|---|
| `Hello { viewer }` | front end → server | register this id |
| `Bye` | front end → server | leave |
| `Error { message }` | server → front end | door failed; connection stays |

Everything else crosses unchanged: `Call`, `Post`, `Subscribe` out; `Reply`, `PostDone`, `Opened`,
`StreamItem` back. One line is one JSON value; the newline is the framing. The cap is 32 MiB.

### Endpoint

One endpoint per database: `$XDG_RUNTIME_DIR/mypi-<fingerprint>.sock`, or a `mypi/` directory under
the system temporary directory. Directory 0700, socket 0600. The fingerprint hashes the database
path, so two databases mean two endpoints.

Unix socket on unix, named pipe on Windows. `config.server.platform.ipc` chooses `os` (default),
`unix`, or `pipe`. Only the unix socket is written.

### Who serves

The database lock decides, not the endpoint. A process takes the file lock before binding; if it is
already held, another process serves and this one is a front end. The kernel drops the lock on
death, so there is no stale lock. The endpoint is removed when the server stops; a leftover endpoint
is tried once, then replaced.

### One watch, one connection

One command connection carries questions, changes, and stretches, one at a time, paired by order.
Each watched session gets its own connection, and the server only writes on it. Dropping the
receiving end closes the connection; the server unwatches.

### Memory

No durable state. The server keeps the lock, endpoint, and registered front ends. A front end keeps
its command connection and, per watch, a channel and reader thread.

### Invariants

1. One line is one value.
2. The database lock decides who serves.
3. A connection that has not said `Hello` is not registered and does not keep the server alive.
4. After a watch, the server never reads that connection again.
5. With `buffered` streaming, a front end sees only whole blocks.

## 2. Intent

- **No HTTP, no port.** One byte stream on one machine; file permissions are the boundary.
- **No invented shapes.** The wire carries the api's own values.
- **Lock on the database.** The kernel drops it; the database says who serves.
- **Front end names itself.** The id is made above this layer and passed to `Client::connect`; there
  is no factory here.
- **One watch, one connection.** Ending a watch is ending a connection.
- **Stopping rule.** The server stops when nobody is registered, after a short grace period.

## 3. Open questions

1. Nothing reconnects after the server dies.
2. The server's log ring has no door.
3. Windows named pipes are not written.
4. No authentication or encryption; local only.
5. One-shot commands (`mypi --sessions`) bypass this layer.

## 4. Config

| key | decides |
|---|---|
| `config.server.platform.ipc` | endpoint kind: `os`, `unix`, `pipe` |
| `config.server.request.streaming` | watch granularity: `immediate` or `buffered` |
