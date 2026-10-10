# storage architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`; what a
future change will have to touch is in `TODO.md`. This file describes the code as it stands, not
how it got here.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations, and what this layer exports |
| `schema.rs` | `prepare(path)` — creates the file and the tables, wipes it on a version mismatch; `SCHEMA_VERSION` |
| `shape.rs` | every value that crosses this layer's boundary: blocks (`Block`, `NewBlock`, `BlockKind`, the `Spill` a block may carry), rows (`SessionMeta`, `TurnRow`, `RoundUsage`, `RequestMeta`, `Artifact`), the writer's own vocabulary (`Submit`, `SessionChange`, `SessionRef`, `Landed`, `Relay`), and the citation codec (`base36`, `parse_base36`) |
| `decode.rs` | the read recipe: the column lists (`BLOCK_COLS`, `TURN_COLS`, `ARTIFACT_COLS`, `BLOCK_BYTES_SQL`) and one decoder per row kind |
| `reader.rs` | `Reader`: how to open a read connection, and how to close it |
| `writer/mod.rs` | `Writer` (the handle) and `Store` (what the thread owns) |
| `writer/job.rs` | one job on the queue (`What`) and the answer it comes back with (`Answer`) |
| `writer/thread.rs` | the thread body: take one job, carry it out, answer |
| `writer/inlet.rs` | what lands when, and what is relayed at once (`Inlet`: the batch counter, the unlanded tail, the blocks still being written, the relay port) |
| `write/batches.rs` | landing a batch: open a round, append a batch (a spilled block's artifact and citation land here), settle a round |
| `write/compaction.rs` | the summary round and the borrowed rounds |
| `write/sessions.rs` | the session row and the cwd history |
| `write/artifacts.rs` | the row a spilling block lands, written inside its batch's transaction |
| `write/shared.rs` | the small SQL the write steps share |
| `delete/mod.rs` | the sweep: rounds, blocks and artifacts that nothing references |
| `delete/rewind.rs` | the prune-mode cut (`Target`, `Cut`) |
| `delete/session.rs` | deleting a whole session |

## The public surface

| item | what it does |
|---|---|
| `Writer::open(path, batch)` | opens (or creates) the database, starts the writer thread; `batch` is the config's number |
| `Writer::push(submit)` | hands one submission over; returns what landed |
| `Writer::relay(port)` | names the port new pieces go to (one port, the api's) |
| `Writer::rewind(target)` | cuts a session's list back and sweeps; returns what survived |
| `Writer::delete_session(session)` | removes a session and everything only it referenced |
| `Reader::open(path)` / `Reader::close(conn)` | a read connection, handed out and taken back |
| `Target`, `Cut` | what a rewind takes, and what survived it |
| `BORROWED` | the stop reason a borrowed round carries |
| `shape::*`, `decode::*` | the values and the read recipe |

Everything else in this folder is `pub(crate)` or private.

## Relationships

```text
        api  ──push / relay / rewind / delete_session──┐
        agent ──push (blocks, archives, round ends)────┤
                                                        ▼
                                    ┌────── writer thread ────────┐
                                    │ Writer handle → Job (queue) │
                                    │ Store: conn + inlet         │
                                    │   → write/*, delete/*       │
                                    └──┬──────────────────────────┘
                                       │ Relay (each piece names
                                       │ its session)
                                       ▼
                                    api → whoever asked for it

        readers (api, agent) ──Reader::open + own SQL + decode::*──► rows
        (the session's state also reads one artifact row by block id, to
         materialize a citation)
```

- **Who calls storage**: api (its doors), the agent side (`push` for a reply's blocks, the request
  archive, the round's end, the tool results).
- **Who storage calls**: nobody of ours. The clock, the paths and the numbers arrive as values;
  only `rusqlite` and `serde_json` are used underneath.
- **Who reads**: anyone, with `Reader` and its own query. Storage answers no query.
- **Who storage does not know**: any front end, the command line, the tools, and the config files.
- **Threads**: one writer thread, created by `Writer::open`. Readers run wherever their caller
  runs. The `Writer` handle is the only thing that crosses between them.

## Parts that must stay in step

1. `decode.rs`'s column constants and the decoders beside them — the decoders read by position.
2. `write/batches.rs`'s insert list and `decode::BLOCK_COLS`.
3. `NewBlock` and `Block` — the conversion between them is `NewBlock::landed`.
4. `write/batches.rs::bytes_of` and `decode::BLOCK_BYTES_SQL` — one measure, written twice (Rust
   and SQL).
5. The citation: `shape::base36` spells a block id, `shape::parse_base36` reads it back, and the
   tool side resolves what the model types with that parser.

## Not wired yet

1. Nothing outside the api calls `Writer::open`: the api's own state opens the database the first
   time one of its doors needs it, at `history.db` under the platform's data directory.
2. Nothing reads an artifact back through this layer: the session's state opens its own connection
   and materializes the row itself (`../agent/ARCHITECTURE.md`).
