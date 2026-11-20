# session — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `actions.rs` | one named action per front-end-facing door: the only place such a door's kind and name are written | `Session`'s public methods |
| `vocabulary.rs` | `ModelId`, `Retention`, `Effort`: the words a front end and the api both use | public |
| `packet.rs` | `Packet { kind, name, content }`, the normalized unit | public |
| `client.rs` | `Client`, `Incoming`: the queue, the matching, the send, the clear | private |
| `session.rs` | `Session`: connect, forward, attach, send, the ingest of each frame | public |
| `content.rs` | `ContentCache`: whole blocks by id, the budget, the eviction | private |
| `structure.rs` | `Turn`, `Structure`: the rounds, their ranges, the cut at one spot | public |

## What it talks to

- **`ipc::client`**: one connection; `send`, `drain`, `alive`.
- **`ipc::contract`**: `Frame` only.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, tables, cards, rounds beyond their ranges, block fields beyond an id, doors' payloads beyond the blocks it answers and the rewind it trims.
- **A front end** names actions, not doors: `switch_model`, `set_cwd`, `submit`. The kind and the door name stay on this side. `post init` is the exception: it opens the session the actions need.

## Inside

```text
Session
 ├── client::Client     ── the queue, the frames still owed an answer
 ├── structure::Structure ── which rounds exist and which blocks they own
 ├── content::ContentCache── the blocks last seen, whole
 └── outbox: Vec<Packet>  ── what arrived since the last forward
```

- `Client::pump` sends what is queued and says what came back: `Push` or `Answer { request, reply }`.
- `Session::take` is the only place a frame reaches the windows: a push updates the structure for the attached session only, an answer updates both windows and the budget, then every frame goes to the outbox.
- `Session::send` is the only outbound path; `Session::blocks` answers a covered range from the content window and sends the rest upstream.
- An accepted `post meta` rewind cuts the structure at the spot its own request named; the reply is forwarded without being decoded.

## Must stay in sync

1. `content::ContentCache` and the `preloadCache` key.
2. `structure::Turn` and the wire's `ranges` and `compact`: pairwise, closed intervals, newest first once loaded.
3. `Session::attach` and both windows: they live exactly as long as one attachment.
4. `Packet` and the front end's `Kind` table: a new upstream kind is a string here and a declaration there.
5. The names this layer matches (`call init`, `call blocks`, `call config`, `post meta`, `post delete`, `stream`, `local`) and api's door table.
6. The rewind spot and the structure cut: read from the request, applied when the answer succeeds.

## Not wired

1. No reconnect: a dead read half leaves one `local / disconnected` and no further frames.