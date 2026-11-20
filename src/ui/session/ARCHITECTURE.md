# session — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `up.rs` | `Up`: the connection, frames out, frames in, whether it lives, this front end's name | private |
| `down.rs` | `Down`: the outbox, and the window a front end borrows | private |
| `session.rs` | `Session`: the identity, attach and detach, send, receive, the ingest of each payload | public |
| `structure.rs` | `Turn`, `Structure`, `ranges_of`: the rounds, their ranges, the snapshot, the cut | public |

## What it talks to

- **`ipc::client`**: one connection; `connect`, `send`, `drain`, `alive`.
- **`ipc::contract`**: `Frame` only.
- **`ipc::endpoint`**: where the server listens; the session does not start it.
- **Knows nothing about**: terminals, zones, block fields beyond an id, doors' payloads beyond the round and the cut.

## Inside

`Session` holds the identity, the two halves, and whether the read half has already reported its death. `Up` writes and drains; `Down` keeps the outbox and the window.

`receive` drains the connection, folds each payload into the window, then hands the frames back whole and in order. `send` fills in the session id and does nothing else.

## Must stay in sync

1. `structure::Turn` and the wire's `ranges` and `compact`: pairwise, closed intervals, oldest round first.
2. The payload shapes the window reads (`structure`, `turn` with `ranges`, `signal: rewind` with `block`) and the api frames that carry them.
3. `Structure::of` and the snapshot a `call init` answer carries.

## Not wired

1. A frame for another conversation is not recognised: one connection follows one conversation.
2. Nothing here asks the window anything: the borrow exists for the front end that wants it.
