# session

One conversation on the wire: a connection up, frames down, and the structural window that goes with it.

## 1. Design / Schema

### The unit

`Session` is one conversation. It speaks `ipc::contract::Frame` in both directions and decodes nothing: `kind`, `name` and `payload` cross unchanged.

### The identity

`open(endpoint, id)` starts bound or unbound. `attach(id)` binds the conversation, `detach()` releases it. While bound, every frame on its way out carries the `session` field, filled in when the writer left it out; while unbound, frames leave exactly as they were written, which is what a draft's opening frame needs.

### Entry points

`open`, `attach`, `detach`, `send`, `receive`, `structure`, `close`.

`send` writes one frame and says nothing about any answer. `receive` hands back everything the connection produced since the last call, in arrival order, each frame whole.

### The window

`Structure` holds the rounds: turn id, closed block ranges, and whether the round is a compaction. It is keyed by turn id, so key order is round order and the newest round comes last.

Three shapes reach it:

| payload | effect |
|---|---|
| carrying `structure` (a `call init` answer) | replaces the window |
| carrying `turn` and `ranges` (a round's receipt) | that round is set |
| carrying `signal: rewind` and `block` | everything after that block goes, and the round owning it keeps only its head |

`attach` and `detach` empty the window: a round belongs to one conversation.

A `local` frame is this layer's own voice: `disconnected`, once, when the read half closes.

## 2. Design intent

- **The postman does not open the mail.** What a front end draws is decoded by whoever draws it.
- **One record of how far the conversation reaches.** A front end may keep its own copy for whatever it re-lays; this one is the authority on rounds.
- **A cut is a block, not a round.** A rewind names the block the user kept, and the server does not decide which round survives.

## 3. Unresolved

1. No reconnect: a dead read half leaves one `local / disconnected` and nothing after it.
2. Frames are not filtered by session id: one connection follows one conversation, and the relay sends it nothing else.

## 4. Config fields

None.
