# storage architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`; this file
describes the code as it stands.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | declarations and exports |
| `schema.rs` | `prepare(path)`, the tables, `SCHEMA_VERSION`; a mismatch rebuilds |
| `shape.rs` | every value crossing the boundary: blocks, rows, request archives, session changes, submissions, effects, relay pieces, citation codec |
| `decode.rs` | column lists (`BLOCK_COLS`, `TURN_COLS`, `ARTIFACT_COLS`, `BLOCK_BYTES_SQL`) and row decoders |
| `reader.rs` | `Reader`: open and close a read connection |
| `writer/mod.rs` | `Writer` handle and `Store` |
| `writer/job.rs` | one queue job and its answer |
| `writer/thread.rs` | the thread body |
| `writer/inlet.rs` | batch size, tool-round count, unlanded tail (blocks and effects), open blocks, relay port |
| `write/batches.rs` | open a round, land a batch (block, spill artifact, effects), settle a round |
| `write/compaction.rs` | summary round and borrowed rounds |
| `write/sessions.rs` | session row, cwd history, profile, todo marker |
| `write/artifacts.rs` | the row a spilling block lands |
| `write/shared.rs` | shared write SQL: turns, list rows, request archives, session block ranges |
| `write/effects.rs` | dispatch an effect to memo, alias or subsession |
| `write/memos.rs` | append-only memo rows |
| `write/aliases.rs` | alias rows and visible-set removal |
| `delete/mod.rs` | the sweep for rounds, blocks, artifacts and orphan effects |
| `delete/rewind.rs` | prune-mode cut (`Target`, `Cut`) |
| `delete/session.rs` | export memos, delete a session, sweep |
| `delete/export.rs` | one markdown memo file per session |

## The public surface

| item | what it does |
|---|---|
| `Writer::open(path, batch)` | opens or creates the database, starts the writer thread |
| `Writer::push(submit)` | hands one submission over; returns what landed |
| `Writer::relay(port)` | names the port live pieces go to |
| `Writer::rewind(target)` | cuts a session's list back and sweeps |
| `Writer::delete_session(session)` | exports memos, removes the session and its anchored rows |
| `Reader::open(path)` / `Reader::close(conn)` | a read connection, handed out and taken back |
| `Target`, `Cut` | rewind input and result |
| `BORROWED` | stop reason of a borrowed round |
| `shape::*`, `decode::*` | values and the read recipe |

Everything else is `pub(crate)` or private.

## Relationships

```text
        api  -- push / relay / rewind / delete_session --> writer thread
        agent -- push blocks, effects, archives, ends --> writer thread
                                                        |
                                                        v
                                            Store: conn + inlet
                                              -> write/*, delete/*
                                                        |
                                                        v
                                            Relay (session + round)
                                                        |
                                                        v
                                            api -> whoever asked

        readers (api, agent) -- Reader::open + own SQL + decode::* --> rows
```

- **Who calls storage**: api (its doors) and the agent side (blocks, effects, archives, round ends).
- **Who storage calls**: nobody of ours. Clock, paths and numbers arrive as values; only `rusqlite`
  and `serde_json` are used underneath.
- **Who reads**: anyone, with `Reader` and its own query.
- **Who storage does not know**: any front end, the command line, the tools, and config files.
- **Threads**: one writer thread created by `Writer::open`; readers run wherever their caller runs.

## Parts that must stay in step

1. `decode.rs`'s column constants and decoders; decoders read by position.
2. `sessions` column order and `decode::session_meta` (`profile` before `todo_on`).
3. `TURN_COLS`, `turn_from_row`, and `request_meta` columns.
4. `write/batches.rs`'s insert list and `decode::BLOCK_COLS`.
5. `NewBlock` and `Block`; `NewBlock::landed` converts.
6. `write/batches.rs::bytes_of` and `decode::BLOCK_BYTES_SQL`.
7. `shape::base36` / `parse_base36` and the citation in `cite`.
8. `write/shared.rs::request_hash` and `put_request_meta`; every field in the hash must be stored.
9. `write/batches.rs::land` and `write/effects.rs`; an effect's `call_id` must name a tool result in
   the same batch.
10. `delete/session.rs`'s anchored deletes and `delete/mod.rs::prune`; both use the same survival
    rule.

## Not wired yet

1. Config computes the profile fingerprint; storage only stores the string.
