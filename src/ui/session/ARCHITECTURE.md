# session — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `actions.rs` | one named action per door: the only place a door's kind and name are written | public |
| `vocabulary.rs` | `ModelId`, `Retention`: the words a front end and the api both use | public |
| `packet.rs` | `Packet { kind, name, content }`, the normalized unit | public |
| `client.rs` | `Client`, `Incoming`: the queue, the matching, the send | private |
| `session.rs` | `Session`: connect, forward, attach, send, the ingest of each frame | public |
| `content.rs` | `ContentCache`: whole rows by id, the budget, the eviction | private |
| `structure.rs` | `Turn`, `Structure`: the rounds, their ranges, the cut at one spot | public |

## What it talks to

- **`ipc::client`**: one connection; `send`, `drain`, `alive`.
- **`ipc::contract`**: `Frame` only.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, tables, cards, rounds beyond their ranges, block fields
  beyond an id, doors' payloads beyond the two special cases below.
- **A front end** names actions, not doors: `switch_model`, `set_cwd`, `submit`. The kind and the
  door name stay on this side.

## Inside

```text
Session
 ├── client::Client     ── the queue, the frames still owed an answer
 ├── structure::Structure ── which rounds exist and which blocks they own
 ├── content::ContentCache── the rows last seen, whole
 └── outbox: Vec<Packet>  ── what arrived since the last forward
```

- `Client::pump` sends what is queued and says what came back:
  `Push`, `Answer { request, reply }`, `Lost { request, reason }`.
- `Session::take` is the only place a frame reaches the windows: a push updates the structure,
  an answer updates both windows and the budget, then every frame goes to the outbox.
- `Session::send` is the only outbound path; `call blocks` is the one door it answers itself.
- A `post rewind` leaves a note in the ledger before its frame goes out; the accepted reply is
  what makes the note land, and the spot comes from the note, not from the reply's `cut`.

## Must stay in sync

1. `content::ContentCache` and the `preloadCache` key.
2. `structure::Turn` and the wire's `ranges`: pairwise, closed intervals, newest first once
   loaded.
3. `Session::attach` and both windows: they live exactly as long as one attachment.
4. `Packet` and the front end's `Kind` table: a new upstream kind is a string here and a
   declaration there.
5. The two doors the session reads by name (`call blocks`, `call config`) and api's door table.
6. The rewind ledger and the structure cut: one entry per request, applied in arrival order.

## Not wired

1. No reconnect: a dead read half leaves one `local / disconnected` and no further frames.
