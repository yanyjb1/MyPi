# ipc

The wire between the two mypi processes: one line, one value, and the api's own shapes in both
directions. Nothing above this layer is known here.

## 1. The design

### Two halves

mypi runs as two processes: a **server**, which owns the database, and a **front end**. They share no
memory; everything between them is this layer.

| half | holds | does |
|---|---|---|
| server | the database lock, the listening endpoint, the register of front ends | accepts connections, answers what they ask, hands out streams, stops when no front end is left |
| front end | one command connection, one connection per watched session | asks, changes, opens stretches, watches sessions, says when it leaves |

`mypi --server` is the server alone. Plain `mypi` decides on the spot: it looks for a server, and if
there is none it takes the database lock, binds the endpoint, starts the server, and spawns a copy of
itself as the front end.

### The wire

Both directions carry the api's own vocabulary. Only three lines are the protocol's own:

| line | direction | means |
|---|---|---|
| `Hello { viewer }` | front end → server | I am here, and this is the id I go by |
| `Bye` | front end → server | I am leaving |
| `Error { message }` | server → front end | the door failed; the connection stays open |

Everything else crosses unchanged: `Call`, `Post` and `Subscribe` out; `Reply`, `PostDone`, `Opened`
and `StreamItem` back. A question is answered by the value the api returned, so a shape change in the
api is a change in one place, not two.

### One line, one value

A message is one line of JSON. A JSON value never holds a literal newline, so the newline is the
whole framing. A line is capped at 32 MiB; a longer one is a protocol error, not a reason to keep
growing a buffer. A connection that ends in the middle of a line is an error, not a message.

### Where the endpoint lives

One endpoint per database: `$XDG_RUNTIME_DIR/mypi-<fingerprint>.sock`, or, when that variable is
unset, a `mypi/` directory under the system's temporary directory. The directory is 0700 and the
socket file 0600, so only this user reaches it. The fingerprint is a short hash of the database's
path: two databases open at once means two endpoints, each with its own server.

### Which kind of endpoint

The endpoint is a unix socket on unix and a named pipe on Windows. Which kind is used is a setting,
`config.server.platform.ipc`, with three values: `os` (the default — whatever this system does),
`unix`, `pipe`. The first asks the platform layer for this system's family; the other two pin one
kind, for debugging. Only the unix socket is written today (§3). This is the one place the operating
system shows through: the wire above it is a byte stream either way.

### Who is the server

The database's lock decides, not the endpoint. A process that wants to serve opens the database file
and takes its lock; if the lock is already taken, another process is serving and this one becomes a
front end. The kernel drops the lock when the process dies, so there is no stale lock to clean up.
The lock is taken **before** the endpoint is bound, and the endpoint is removed when the server
stops. A leftover endpoint is tried once, then replaced.

### One watch, one connection

A front end holds one command connection: questions, changes and stretches all go over it, one at a
time, paired by order — there are no request ids. Watching is separate: each watched session gets its
own connection, and on that one the server only writes.

A watch ends with its connection. A front end stops watching by dropping the receiving end (its
reader thread then exits and the connection closes); a front end that dies closes it too. Either way
the server unwatches, and the session's own state is let go by the api's usual rules.

### What lives in memory

Nothing durable, and no session state. The server keeps the lock it holds, the endpoint it listens
on, and the count of front ends that registered. A front end keeps its command connection and, per
watched session, a channel and the thread reading it. Everything else is in the database or in the
config.

### Invariants

1. One line is one value; a message never spans lines.
2. The database's lock decides who serves; the endpoint does not.
3. A connection that has not said `Hello` is not registered, and does not keep the server alive.
4. After a watch, the server never reads that connection again.
5. With `buffered` streaming a front end is shown only whole blocks — a piece it is not shown lands
   in the database anyway, so nothing is lost by dropping it.

## 2. Design intent

**No HTTP, no port.** The two processes are on the same machine, and one byte stream between them
carries everything. There is no port to pick, no route to name, and the endpoint's own file
permissions are the boundary — which is also why this layer needs no authentication while it stays
local.

**The wire invents no shapes.** Questions, changes and streams cross as the api's own values, so a
front end learns one vocabulary, not two, and a shape changes in one place.

**The lock lives on the database file.** The kernel drops a file lock when the process dies, so
nothing goes stale, and the database is what says who serves it.

**The front end makes its own id.** Two surfaces may watch one session, and each names itself, so
neither steals the other's pieces — the api's streams are already per viewer, and this layer just
carries that id through.

**One command connection, one connection per watch.** Questions are paired by order, which removes
request ids and any need for multiplexing. A watch is a connection, so ending a watch is ending a
connection — no separate message to send, and a front end that dies is noticed either way.

**The register, plus a grace period, is the whole stopping rule.** The server stops when nobody is
registered, and waits a moment before believing it, because a front end is started before it can
register. It never asks whether a round is running: a round with no watcher is the api's business,
and its content is in the database either way.

**The streaming setting is read here, not in each front end.** Whether a watch carries every decoded
chunk or only whole blocks is one value, and the choice is made where the stream is written. A front
end asks to watch; it does not choose the granularity.

**A watch needs no bookkeeping to end.** The stream's life is its connection's life, so nothing has
to be remembered, timed out, or cleaned up after a crash.

### Reserved: future products that already shaped this

| future thing | what it already decided |
|---|---|
| Windows named pipes | the endpoint kind is a setting, and the platform layer is the seam; a pipe name is not a file, so "remove the endpoint" is a no-op there |
| A second edge for remote front ends | the wire carries the api's shapes as they are, so another edge only translates; this layer holds no port and no route |
| Reconnecting after the server dies | a front end holds no session state of its own, so a fresh connection and a fresh watch rebuild everything it needs |

## 3. Open questions

1. Nothing reconnects. When the server dies, a front end's command connection fails and its watches
   end; the client has no "broken" state and no retry.
2. A front end cannot read the server's log. The log is a ring inside the server process, and no door
   exposes it.
3. The Windows named pipe is not written: the platform layer's Windows side is `unimplemented!()`.
4. There is no authentication and no encryption. That is fine while the endpoint stays on this
   machine; a remote edge needs an answer of its own.
5. One-shot commands do not use this layer at all (`mypi --sessions` opens its own read connection),
   so printing a table needs no server. Whether other read-only commands should do the same is
   undecided.

## 4. Config

Read through `Config`; this layer opens no file.

| key | what it decides |
|---|---|
| `config.server.platform.ipc` | which endpoint kind to use: `os` (this system's), `unix`, `pipe` |
| `config.server.request.streaming` | what a watch carries: `immediate` = every decoded chunk, `buffered` = one piece per finished block |

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
