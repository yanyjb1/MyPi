# storage

The database, its only writer and its only reader. Nothing above this layer is known here.

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
| `sessions` | one row per session: `id`, `name`, `started_at`, `cwd`, `model_provider`, `model`, `todo_on` |
| `session_turns` | the session's round list: `session_id`, `idx`, `turn_id`, `is_compaction`. The newest `is_compaction` row is where reading starts |
| `turns` | one row per round: `turn_id`, `request_id`, `blocks` (one `[start, end]` per landed batch), `stop_reason`, four usage columns |
| `request_meta` | one row per distinct request archive, deduplicated by hash; `profile` names the profile it was built from |
| `blocks` | one row per message: the id, `type`, body and injection, six usage columns, tool columns, `memo`, `duration_ms`, `ts` |
| `cwd_history` | round-anchored working directories |
| `artifacts` | oversized tool output: one row per block that spilled; `block_id` is the key |
| `memos` | append-only memo revisions: `anchor` (`NULL` = global), `name`, `content`, `ts` |
| `aliases` | alias rows: `anchor`, `name`, `command`, `ts` |
| `subsessions` | child session id -> the block that spawned it |

### The write path

- `Writer::push(Submit)` is the only way in. One submission is one transaction: its rows run in
  order, and either all land or none do.
- A row names its table and the fields it writes: insert a session, a round, a request archive, a
  block, an artifact, a memo, an alias, a link, a directory entry, a list place; rename, switch,
  move, mark, settle. Storage reads no meaning into a row.
- A row that needs another row's id names it: a known number, or an earlier row of the same
  submission (the id exists only after that row runs).
- The writer fills what only it can: auto-increment ids, the round's interval and byte count when a
  block lands, the request archive's hash and its deduplication.
- `Writer::relay(port)` names where the round structure goes: when a round settles, its id and the
  closed id ranges its blocks occupy. Nothing unlanded is relayed, and no block text is repeated.
- Deletion is one door: `delete_rows(table, ids)` — the rows, whatever only they held, then the
  same sweep. A session's deletion is its row; the memo export is its own queued job.
- A rewind is not row deletion: it rewrites a round's interval, then deletes the tail rows and
  sweeps.

### The read side

`Reader` owns one read connection, every query, and every decode. A caller asks for one table and
gets JSON rows under the schema's field names. Stored shapes never leave: interval JSON becomes an
array, `0`/`1` becomes a boolean, JSON in a TEXT column becomes JSON. The path and the column lists
never leave storage either.

### What lives in memory

The structure port, and nothing else.

### Invariants

1. A session's list is the only authority on its context, and its order is the order.
2. A round's index is the only authority on which blocks it holds.
3. One submission, one transaction.
4. Rows are written once; later changes are their own rows.
5. Reads never walk a chain, never clamp, never recurse.
6. A schema mismatch rebuilds the database; there is no migration.

## 2. Design intent

**One writer.** The connection has exactly one owner, so writes queue instead of colliding.

**One entry, no exceptions.** Every write is a row: a table and its fields. There is no per-table
or per-tool branch, so a new caller never needs a new door, and storage never learns what a tool
meant.

**Timing belongs to the caller.** Storage writes when it is told to and does not decide when a
batch is due.

**Ids the caller cannot know are filled here.** Auto-increment keys and everything derived from
them — the round's interval, the archive's hash — are the writer's business.

**Profile is not stored.** The archive records the profile each request was built from; a session
carries no profile of its own.

**Blocks are columns.** Readers take the columns they need; SQL can filter and aggregate on real
columns.

**Memo and alias rows are just rows.** Which version is current, whether a name exists, what is
visible from a session: the caller's judgement, made on rows it has read.

## 3. Open questions

None.

## 4. Config

Storage reads no config file, and takes no config value.
