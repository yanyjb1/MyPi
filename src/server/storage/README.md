# storage

The database, and the one writer that touches it. Nothing above this layer is known here.

## 1. The design (and the schema)

### Three levels

```
session → its round list (a table) → rounds → their block index (intervals) → blocks
```

Each level holds the one below by index; a block references nothing. The DDL in `schema.rs` is the
source of truth for every column.

### The tables

| table | holds |
|---|---|
| `sessions` | one row per session: `id`, `name`, `started_at`, `cwd`, `model_provider`, `model`, `todo_on` |
| `session_turns` | the session's round list, one row per item: `session_id`, `idx`, `turn_id`, `kind`, `is_compaction` (two indexes). The newest `is_compaction` row is where reading starts |
| `turns` | one row per round: `turn_id`, `request_id`, `blocks` (the round's block index — one `[start, end]` per landed batch), `stop_reason` (`rewind` / `compaction` / `borrowed`), `bytes`, five usage columns |
| `request_meta` | one row per distinct request archive, deduplicated by hash |
| `blocks` | one row per message, twenty columns: the id, `type`, the body and its injection (`content`, `prefix`, `suffix`), six usage columns, the tool columns (`call_id`, `name`, `args`, `intent`, `text`, `first`, `ok`, `result`), `memo`, `duration_ms`, `ts` |
| `cwd_history` | the round-anchored directories a session has worked in |
| `artifacts` | oversized tool output: one row per block that spilled, `block_id` is the key (the block's own `result` text ends with the citation), swept with the block |

### The write path

- `Writer::push(Submit)` is the only way in: one thing at a time, each naming the table it lands in.
  The answer says what landed.
- The handle is cheap to clone and safe to hand to any thread: it puts the submission on a channel.
  One thread takes them off, owns the connection, and lands them. Submission order is landing
  order, and exactly one writer ever touches the database.
- `Writer::relay(port)` names where new pieces go: one port, the api's. Every piece carries its
  session, and the api hands each piece to whoever asked for that session.
- Every round ends with one relayed piece: a loop round when its end lands, a compaction when the
  summary lands. The reason it carries is the one the round's own row carries.
- A block can be handed in **before it is finished** (`done: false`): the writer keeps it, relays
  every piece the moment it arrives, and does not write it. A finished block of the same kind (and
  the same tool call) closes it — the request side's cut is the authoritative version. The piece
  carries that same flag, so a front end can tell an increment from the finished block. A round that
  ends with a block still open lands it as it stands, so nothing is ever held forever.
- Three things land a batch: a user's message, every N tool rounds, the end of a round — the last
  one always.
- A block may carry the **full text of an oversized tool output** (`NewBlock::spill`, decided by the
  tool side): landing it writes the block, the artifact row (`block_id` = the block's id, one row
  per block) and the citation appended to the result text, all in one transaction. The citation is
  `#<ref>`, the block id in base36 (`shape::base36`); it is frozen with the row, so a rewind or a
  fork leaves it alone and a prefix cache stays warm.
- The batch counter lives in the writer thread; `N` is handed in.
- A rewind and a session's deletion are writes too: they go through the same queue.

### The read recipe

`Reader::open(path)` hands out a connection and `Reader::close` takes it back. A reader writes its
own SQL and decodes the rows with `decode`'s column lists (`BLOCK_COLS`, `TURN_COLS`,
`ARTIFACT_COLS`, `BLOCK_BYTES_SQL`) and one decoder per row kind. Storage answers no query.

### What lives in memory

Four things, and only these, all inside the writer thread: the batch counter, the tail handed in
but not landed yet, the blocks a round is still writing, and the relay port.

### Invariants

1. A session's list is the only authority on its context, and its order is the order.
2. A round's index is the only authority on which blocks it holds.
3. Rounds, blocks and request archives are written once and never edited (a spilled block's
   citation is added inside the transaction that inserts it, so no reader ever sees the row
   without it).
4. Reads never walk a chain, never clamp, never recurse.

## 2. Design intent

**One writer.** The connection has exactly one owner, so two rounds writing at once queue up
instead of colliding, and no caller spells out SQL for a write.

**Timing lives here.** When a batch lands is a property of the write path, not of each caller, so
the rule and the counter sit in one place. `N` is handed in, never read here.

**Reading is the reader's job.** A reader opens its own connection: the writer's work never locks a
surface's reads out, and this layer stays below everything else.

**Relaying is immediate, writing is batched — and a block is written only once it is finished.** A
surface sees text the moment it exists; the disk sees whole blocks, in batches.

**Blocks are columns.** A reader takes exactly the columns it needs instead of decoding a whole
blob and filtering in Rust, and SQL can filter and aggregate on real columns (`type`, `call_id`,
the usage numbers).

**Snapshot rebuild, no migrations.** A version mismatch wipes and rebuilds the file.

**State has two homes.** The database's rows and the config files — anything either can answer, we
keep no third copy of. The three things in memory are the residue neither can carry.

**Reserved: future products that already shaped this.**

| future thing | what it already decided |
|---|---|
| Fork mode (a rewind that branches instead of cutting) | the list has a `kind` column, and the fork's SQL is written down; today the mode panics |
| Instant-messaging mode (a timestamp injected around each user message) | blocks carry `prefix` / `suffix` and a `ts`, so injection is frozen at write time and "when did it land" is answerable |
| A front end that draws paired diffs | blocks carry a `memo` column, for what only the tool can compute |

## 3. Open questions

1. How fast the block columns read and write, and the shape of the block-fetch query (one query
   per interval, or intervals into a temp table and one join) — both deferred.
2. Fetching an artifact stays the caller's own query: the session's state reads the row by
   `block_id` and materializes it (`../agent/README.md`).

## 4. config

Storage reads no config file itself. Three keys decide how it behaves; whoever calls it reads them
and hands the values over.

| key | what it decides | how it reaches storage |
|---|---|---|
| `config.server.storage.batchTools` | how many completed tool rounds land one batch (default 10; `0` is rejected when the config loads) | the second argument of `Writer::open` |
| `config.server.request.streaming` | how much of a running round a front end sees: `immediate` = every decoded chunk, `buffered` = one piece per finished block | read at the backend-to-front-end edge (`src/ipc/stream.rs`), not here |
| `config.server.pruneOnRewind` | what a rewind does: `true` cuts the list; `false` (keeping what follows as a branch) is not built, and the config load refuses it | nowhere: the config load checks it, and the rewind reads the list, not the key |

Nothing else comes from there: no path, no clock, no limits.

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
`TODO.md` — what a future change will have to touch.
