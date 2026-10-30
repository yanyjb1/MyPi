# storage architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`; this file
describes the code as it stands.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | declarations and exports |
| `path.rs` | the database's file name and its default location |
| `schema.rs` | `prepare(path)`, the tables, `SCHEMA_VERSION`; a mismatch rebuilds |
| `shape.rs` | every value crossing the boundary: blocks, rows, request archives, submissions, references, the round structure, citation codec |
| `decode.rs` | column lists (`SESSION_COLS`, `TURN_COLS`, `REQUEST_COLS`, `BLOCK_COLS`) and row decoders, private to the reader |
| `reader.rs` | `Reader`: one connection, one accessor per table, rows out as JSON |
| `writer/mod.rs` | declarations and exports |
| `writer/handle.rs` | the `Writer` handle and what the thread owns |
| `writer/job.rs` | one queue job and its answer |
| `writer/thread.rs` | the thread body |
| `write/apply.rs` | one submission as one transaction: row dispatch, reference resolution, the structure it reports |
| `write/sessions.rs` | session rows and the cwd history |
| `write/turns.rs` | round rows and their usage columns |
| `write/blocks.rs` | block rows, and the round's interval and byte count |
| `write/artifacts.rs` | the row an oversized output lands |
| `write/memos.rs` | memo rows |
| `write/aliases.rs` | alias rows |
| `write/shared.rs` | the list tail, the session of a round, request archives and their hash |
| `delete/mod.rs` | declarations and exports |
| `delete/sweep.rs` | the one sweep: dead list places and trail entries, rounds, blocks, anchored rows |
| `delete/rewind.rs` | prune-mode cut (`Target`, `Cut`) |
| `delete/rows.rs` | `delete_rows`: one table, the rows, whatever only they held, then the sweep |
| `delete/export.rs` | one markdown memo file per session |

## The public surface

| item | what it does |
|---|---|
| `Writer::open(path)` | opens or creates the database, starts the writer thread |
| `Writer::push(submit)` | one transaction; returns one id per row |
| `Writer::relay(port)` | names the port live pieces go to |
| `Writer::rewind(target)` | cuts a session's list back and sweeps |
| `Writer::delete_rows(table, ids)` | removes those rows and whatever only they held; returns how many |
| `Writer::export_memos(session)` | writes this session's memos into the download folder |
| `Reader::open(path)` | opens the read connection; every query lives behind it |
| `Reader::sessions / session / list / turns / turn / request / blocks / block / artifact / artifacts_by_ref / memos / aliases / cwd_trail` | the reads, decoded: rows out as JSON |
| `Submit`, `Row`, `Ref`, `Table` | one submission, its rows, how a row names another row's id, and what `delete_rows` may remove |
| `Target`, `Cut` | rewind input and result |
| `shape::*`, `decode::*` | values and the row decoders, internal to the reader |

Everything else is `pub(crate)` or private.

## Relationships

```text
        api  -- push / relay / rewind / delete_rows / export --> writer thread
        agent -- push rows ---------------------------> writer thread
                                                        |
                                                        v
                                            Store: conn + relay port
                                              -> write/*, delete/*
                                                        |
                                                        v
                                            Structure (turn,
                                             its block id ranges)
                                                        |
                                                        v
                                            api -> whoever asked

        readers (api, agent) -- Reader::* --> JSON rows
```

- **Who calls storage**: api (its doors) and the agent side (rows).
- **Who storage calls**: nobody of ours. Clock, paths and numbers arrive as values; only `rusqlite`
  and `serde_json` are used underneath.
- **Who reads**: anyone, through `Reader`; nobody else writes SQL or decodes a stored shape.
- **Who storage does not know**: any front end, the command line, the tools, and config files.
- **Threads**: one writer thread created by `Writer::open`; readers run wherever their caller runs.

## Parts that must stay in step

1. `decode.rs`'s column constants and decoders; decoders read by position.
2. `sessions` column order and `decode::session_meta`.
3. `TURN_COLS`, `turn_from_row`, and `request_meta` columns.
4. `write/blocks.rs`'s insert list and `decode::BLOCK_COLS`.
5. `NewBlock` and `Block`; `NewBlock::landed` converts.
6. `write/blocks.rs`'s interval append and `turns::set_blocks`; one encoding, `decode::encode_blocks`.
7. `shape::base36` / `parse_base36`; a citation names a block id.
8. `write/shared.rs::request_hash` and `put_request_meta`; every field in the hash must be stored.
9. `shape::Row` and `write/apply.rs`'s dispatch; a new row is one arm on each side.
10. `delete/rows.rs`'s per-table removals and `delete/mod.rs::sweep`; both use the same survival
    rule.

## Not wired yet

1. `Writer::open` no longer takes the batch size: `config.server.storage.batchTools` has no reader.
