# storage

The database and its only writer. Nothing above this layer is known here.

## 1. The design (and the schema)

### Three levels

```
session -> its round list (a table) -> rounds -> their block index (intervals) -> blocks
```

Each level holds the one below by index; a block references nothing. `schema.rs` is the source of
truth for every column.

### Tables

| table | holds |
|---|---|
| `sessions` | one row per session: `id`, `name`, `started_at`, `cwd`, `model_provider`, `model`, `profile`, `todo_on` |
| `session_turns` | the session's round list: `session_id`, `idx`, `turn_id`, `kind`, `is_compaction`. The newest `is_compaction` row is where reading starts |
| `turns` | one row per round: `turn_id`, `request_id`, `blocks` (one `[start, end]` per landed batch), `stop_reason`, `bytes`, five usage columns |
| `request_meta` | one row per distinct request archive, deduplicated by hash; `profile` names the profile it was built from |
| `blocks` | one row per message: the id, `type`, body and injection, six usage columns, tool columns, `memo`, `duration_ms`, `ts` |
| `cwd_history` | round-anchored working directories |
| `artifacts` | oversized tool output: one row per block that spilled; `block_id` is the key |
| `memos` | append-only memo revisions: `anchor` (`NULL` = global), `name`, `content`, `ts`; identity is `scope + name`, the newest row is current, and `anchor` is provenance |
| `aliases` | alias rows: `anchor`, `name`, `command`, `ts`; no update, the highest `id` in the visible set wins |
| `subsessions` | child session id -> the block that spawned it |

### The write path

- `Writer::push(Submit)` is the only way in. One thread owns the connection; submissions land in
  order.
- `Writer::relay(port)` names where live pieces go. Every piece carries its session and round.
- A block may arrive before it is finished (`done: false`): the writer relays each piece and keeps
  it in memory. A finished piece of the same kind and `call_id` closes it.
- Three things land a batch: a user message, every N tool rounds, the end of a round.
- A tool result may carry `Effect`s. They are applied in the same transaction as the block, so a
  memo, alias or subsession never exists without its block.
- A compaction lands its summary block with the profile fingerprint in `memo`, then one borrowed
  round per retained stretch.
- `set_todo_on` writes the session flag and the `todo` tool result's `memo` (`true` / `false`)
  together. If the block is still in memory, the marker rides it.
- `delete_session` exports current memos first, then deletes the session, its anchored effect rows
  and everything only it referenced. Global memos survive.
- Rewinds and deletions go through the same queue.

### The read recipe

`Reader::open(path)` hands out a connection; `Reader::close` takes it back. A reader writes its own
SQL and decodes with `decode`'s column lists and row decoders. Storage answers no query.

### What lives in memory

Five things, all in the writer thread: the batch size, the tool-round count, the unlanded tail
(blocks and effects), the blocks a round is still writing, and the relay port.

### Invariants

1. A session's list is the only authority on its context, and its order is the order.
2. A round's index is the only authority on which blocks it holds.
3. Rounds, blocks and request archives are written once. Only the todo marker and memo revisions
   are explicit exceptions.
4. A block and its effects share one transaction.
5. Reads never walk a chain, never clamp, never recurse.
6. A schema mismatch rebuilds the database; there is no migration.

## 2. Design intent

**One writer.** The connection has exactly one owner, so writes queue instead of colliding.

**Timing lives here.** When a batch lands is a property of the write path, not of each caller.

**Reading is the reader's job.** A reader opens its own connection; storage stays below every
surface.

**Blocks are columns.** Readers take the columns they need; SQL can filter and aggregate on real
columns.

**Profile is a snapshot.** A session stores the profile name; every request archive stores the
profile it was built from. Compaction stores the resolved content's fingerprint in the summary
block's `memo`. Storage stores strings, never resolves profiles.

**Effects are part of the block.** Memo, alias and subsession writes are tool result effects, so
they cannot drift from the block that caused them.

**Memo history is append-only.** Identity is `scope + name`; `anchor` is provenance. `Add` checks the name is free, `Mod` appends a version, `Del` removes every version. Readers pick the newest row.

**Aliases are positional.** Each `Add` inserts under its block; `Remove` deletes the rows visible
from the session. There is no tombstone and no update.

**Todo is a marker, not a list.** The front end owns the list; storage stores `todo_on` and writes
`true` / `false` into the `todo` tool result's `memo`.

## 3. Open questions

1. The block-fetch query shape: one query per interval, or intervals into a temp table and one
   join. Both are deferred.
2. Artifact fetching stays the caller's query, keyed by block id.

## 4. Config

Storage reads no config file. Callers hand in the values.

| key | what it decides | how it reaches storage |
|---|---|---|
| `config.server.storage.batchTools` | completed tool rounds per batch | `Writer::open`'s second argument |
| `config.server.request.streaming` | how much of a running round a surface sees | read at the backend-to-front-end edge, not here |
| `config.server.pruneOnRewind` | whether a rewind cuts or branches | the config load checks it; the rewind reads the list |

Nothing else comes from there: no path, no clock, no limits.

## See also

`ARCHITECTURE.md` - the files, the relationships, and what is not wired yet.
