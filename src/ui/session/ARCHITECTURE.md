# session — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the two re-exports | public |
| `packet.rs` | `Packet { kind, name, content }`, the normalized unit | public |
| `client.rs` | `Client`, `Incoming`: the queue, the matching, the send | private |
| `session.rs` | `Session`: connect, forward, attach, send, the ingest of each frame | public |
| `content.rs` | `ContentCache`: whole rows by id, the budget, the eviction | private |
| `structure.rs` | `Turn`, `Structure`: the rounds and their ranges, the rewind trim | public |

## What it talks to

- **`ipc::client`**: one connection; `send`, `drain`, `alive`.
- **`ipc::contract`**: `Frame` only.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, tables, cards, rounds beyond their ranges, block fields
  beyond an id, doors' payloads beyond the two special cases below.

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

## Must stay in sync

1. `content::ContentCache` and the `preloadCache` key.
2. `structure::Turn` and the wire's `ranges`: pairwise, closed intervals, newest first once
   loaded.
3. `Session::attach` and both windows: they live exactly as long as one attachment.
4. `Packet` and the front end's `Kind` table: a new upstream kind is a string here and a
   declaration there.
5. The two doors the session reads by name (`call blocks`, `call config`) and api's door table.

## Not wired

1. Nothing asks the session to stop watching; the server has no unwatch door.
2. No reconnect: a dead read half leaves one `local / disconnected` and no further frames.
