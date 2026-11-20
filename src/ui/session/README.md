# session

The front end's half of the wire: one connection, two windows, one outbox.

## 1. Design / Schema

### The unit

Every frame crosses this layer as three parts: `kind`, `name`, `content`. Nothing is decoded into a type of its own.

### Outbound doors

The api takes `name`, `cwd`, `switch`, `effort`, and `rewind` inside one `post meta` frame: `{name, content}`, with the attached session in `content`. Every other door keeps its own name. `actions.rs` is where this layer's doors are named; the frames it originates itself — the `call config` that sizes the window, the `post mailbox` a detach clears, the `register bye` — are queued there too, through `clear_remote_mailbox` and `sign_off`.

### The two windows

| window | holds | keyed by |
|---|---|---|
| `Structure` | the rounds: turn id, closed block ranges, and whether the round is a compaction, newest first | turn id |
| `ContentCache` | whole blocks, exactly as an answer carried them | block id |

- Content budget: `config.ui.session.preloadCache`, asked on every attach through `call config`; its answer feeds the window and is not forwarded.
- Eviction: the earliest admission goes first.
- A pushed stream frame carrying `turn` and `ranges` sets that round, compaction mark included.
- A `call init` answer replaces both windows; a `call blocks` answer merges blocks; an accepted `post delete` answer for the attached session detaches; an accepted `post meta` rewind trims both windows to the spot its own request named.

### The rewind

A `post meta` rewind names its spot in the request. When the answer carries no error, the structure is cut at that spot and the blocks outside it are dropped. Nothing is kept between the request and the answer: the request is the record, and a refused answer changes nothing. The reply is forwarded untouched.

### Config answers

A `call config` answer is forwarded with the key it answers: `{key, value}`. This layer's own `preloadCache` question is swallowed.

### The outbox

`forward()` returns everything since the last call, in arrival order: pushes, answers, and the session's own `local` frames. A frame nobody waits for is forwarded whole as a push, not dropped. `local` is the session's own voice: `disconnected` (the read half closed, once). A `local` frame from the wire, such as the server's `opened`, is forwarded untouched.

### Entry points

```text
connect(endpoint)                  forward() -> Vec<Packet>
attach(id)  detach()  attached()   structure() -> Structure
send(kind, name, content, answer)  bye()
```

Every door has one named action in `actions.rs` (`init`, `meta`, `usage`, `pools`, `pools_at`, `config`, `blocks`, `sessions`, `memo_list`, `memo_history`, `memo_write`, `memo_delete`, `watch`, `unwatch`, `submit`, `interrupt`, `write_mailbox`, `clear_mailbox`, `rename`, `set_cwd`, `switch_model`, `set_default_model`, `set_effort`, `compact`, `delete`, `rewind`, and the layer's own `clear_remote_mailbox` and `sign_off`). The door's kind and name live there; a front end asks for the action, never for the door, except `post init`, which a front end sends through `send` because the session it opens has no id yet. Each session-bound action fills the attached session in unless the caller named one.

- `call blocks` is answered from the content window when it covers the ranges; otherwise the frame goes upstream and the answer is forwarded like any other.
- `answer = false` for frames whose answer nobody waits for (`subscribe watch`, `bye`, the mailbox a detach writes).

### Invariants

1. A frame is forwarded whole; the windows only read what they cannot be kept without.
2. A reply is matched to the oldest queued frame with the same `kind` and `name`; a frame that matches nothing is forwarded whole, not dropped.
3. Both windows live exactly as long as one attachment.
4. The rewind spot is read from the request, never from the reply.

## 2. Design intent

- **The postman does not open the mail.** What the front end draws is decoded by whoever draws it; this layer only keeps the two indexes every reader would otherwise have to rebuild.
- **Answer from the window.** A block read the window already covers costs no round trip.
- **Time before sequence.** The block admitted longest ago is the one the reader moved away from; admission stamps are unique, so id order never decides.

## 3. Unresolved

1. A refused `call blocks` leaves the window untouched; the refusal is forwarded as it came.
2. `post meta` rewind is the one write this layer special-cases: the spot is read out of the request. Every other window change is read from the answer or a push.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.session.preloadCache` | blocks kept in the content window; asked on every attach |