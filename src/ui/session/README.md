# session

The front end's half of the wire: one connection, two windows, one outbox.

## 1. Design / Schema

### The unit

Every frame crosses this layer as three parts: `kind`, `name`, `content`. Nothing is decoded into
a type of its own, and nothing is rewritten on the way out.

### The two windows

| window | holds | keyed by |
|---|---|---|
| `Structure` | the rounds: turn id plus closed block ranges, newest first | turn id |
| `ContentCache` | whole block rows, exactly as an answer carried them | block id |

- Content budget: `config.ui.session.preloadCache`, asked once at connect through `call config`;
  its answer feeds the window and is not forwarded.
- Eviction: the earliest admission goes first, then the lowest id.
- A pushed stream frame carrying `turn` and `ranges` sets that round.
- A `call init` answer replaces both windows; a `call blocks` answer merges rows; a `post delete`
  answer for the attached session detaches.
- A push whose `content.session` is not the attached one is dropped.

### The rewind ledger

A `post rewind` is noted before its frame leaves: the spot it names (a round and a block) is the
whole of what this layer needs. When an accepted reply comes back the structure is cut at that
spot and the rows outside it are dropped. A refused reply drops the note and changes nothing. The
reply's own `cut` is forwarded untouched: what survived belongs to the zone that asked.

### Config answers

A `call config` answer is forwarded with the key it answers: `{key, value}`. Three readers share
one bus, and the request's key is the only thing that tells them apart. This layer's own
`preloadCache` question is swallowed here and never reaches the front end.

### The outbox

`forward()` returns everything since the last call, in arrival order: pushes, answers, and the
session's own `local` frames. `local` is the session's own voice: `lost` (a queued frame broke)
and `disconnected` (the read half closed, once).

### Entry points

```text
connect(endpoint)                  forward() -> Vec<Packet>
attach(id)  detach()  attached()   structure() -> Structure
send(kind, name, content, answer)  bye()
```

Every door has one named action in `actions.rs` (`init`, `meta`, `usage`, `pools`, `config`,
`blocks`, `sessions`, `watch`, `submit`, `interrupt`, `write_mailbox`, `clear_mailbox`, `rename`,
`set_cwd`, `switch_model`, `set_default_model`, `compact`, `delete`, `rewind`). The door's kind and
name live there; a front end asks for the action, never for the door. Each action fills the attached
session in unless the caller named one, and expects a reply unless it is `watch`.

- `call blocks` is answered from the content window when it covers the ranges; otherwise the
  frame goes upstream and the answer is forwarded like any other.
- `answer = false` for doors the server does not reply to (`subscribe watch`, `bye`).

### Invariants

1. A frame is forwarded whole; the windows only read what they cannot be kept without.
2. A push belongs to its session; another session's push is dropped.
3. A reply is matched to the oldest queued frame with the same `kind` and `name`; a mismatch is
   reported as `local / lost`.
4. Both windows live exactly as long as one attachment.
5. The rewind ledger is written from the request, never read out of the reply.

## 2. Design intent

- **The postman does not open the mail.** What the front end draws is decoded by whoever draws
  it; this layer only keeps the two indexes every reader would otherwise have to rebuild.
- **Answer from the window.** A block read the window already covers costs no round trip.
- **Time before sequence.** The row admitted longest ago is the one the reader moved away from;
  ids only break ties.

## 3. Unresolved

1. A refused `call blocks` leaves the window untouched; the refusal is forwarded as it came.
2. `post rewind` is the one door this layer special-cases: the spot is read out of the request and
   held, and the index is trimmed on the answer. Every other window change is read from the answer
   instead. The asymmetry is deliberate for now and the door is expected to be redesigned.
3. A frame lost in the queue leaves its ledger entry behind, so a later rewind trims to the spot
   of the frame that never landed.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.session.preloadCache` | rows kept in the content window; asked once at connect |
