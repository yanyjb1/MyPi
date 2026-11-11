# api

The backend's only front-facing surface. One connection table, one stream per session, and one
door per name. Nothing here knows what a block means; it moves rows and frames.

## 1. Design / Schema

### The connection table

Every front end opens one connection and says `hello` with a viewer id. That id is what the
session's writer slot is keyed by. A connection follows at most one session at a time; `watch`
switches which one, and `watch` with no session stops following. A session nobody follows any
longer is unbooked: its booking, its job schedules and its browser tabs go with it.

### Frames

One frame is `kind`, `name`, `payload`.

| kind | direction | name | payload |
|---|---|---|---|
| `register` | front end | `hello` / `bye` | the viewer id |
| `call` | front end | a read door | the door's arguments |
| `post` | front end | a write door | the door's arguments |
| `subscribe` | front end | `watch` / `submit` / `compact` | the door's arguments |
| `stream` | back end | `change` / `notice` / the door that opened the turn | the pushed row |

A reply carries the request's `kind` and `name`, so a front end matches an answer to its own
request without a counter. A failure is a reply whose payload is `{"error": {"code", "message"}}`.

### Doors

Read: `init`, `blocks`, `meta`, `usage`, `todo`, `memo`, `children`, `config`, `pools`, `sessions`.
Write: `init`, `name`, `cwd`, `switch`, `delete`, `interrupt`, `rewind`, `default_model`, `mailbox`.
Subscribe: `watch`, `submit`, `compact`.

- `init` (post) writes the `sessions` row and its `request_meta` archive in one submission, then
  books the session. The archive is written exactly once per session; the model may change later,
  the archive may not. A child's archive is written by the loop, in the same submission as the
  child's session row.
- `init` (call), `blocks`, `meta`, `memo`, `children`, `sessions` read through `storage::Reader`
  only. The api holds no SQL and decodes no column.
- `todo` answers the whole list `Loop::todo` replayed from the session's own `todo` calls.
- `usage` groups a session's turns by the `provider` and `model` each turn recorded, names each
  group by its `provider:model` address, orders the groups by their token share, and quotes each
  one with `models.yml`'s price in its own currency plus both display currencies. It is live by
  default: the running turn's partial usage comes from the loop's memory and is priced at the model
  in use, so one answer serves the cost pill, the usage panel and the context gauge.
- `usage` also answers where the context stands. While a turn runs, that is the last lap's prompt
  plus its completion; idle, it is the newest turn that measured one. A compaction is the exception:
  the next prompt is the summary plus the retained tail, so `usage` estimates it from the truncation
  point — `context` is the summary's completion plus each cut turn's own added tokens weighted by
  how much of its intervals the tail kept, plus the archive's floor — and says so with `estimated`.
  `baseline_prompt` is that floor: the smallest first-turn prompt among the sessions that share
  this request archive, which is what a front end shows before the first request lands.
- `submit` and `compact` open a turn; a session already in a turn answers `busy`. Nothing is queued
  by the api: a front end that wants a sentence to wait writes it to `mailbox` instead.
- `mailbox` is the session's one pending message. `text` present and non-empty writes it whole over
  whatever was there; no `text` clears it. The loop takes it at a blocking point and opens a turn
  named `mailbox`, exactly as a `submit` would; taking it clears it and puts `signal: mailbox` on
  the stream for every watcher.
- `interrupt`, `cwd`, `switch`, `delete`, `rewind` act on the running session or its rows.
  `interrupt` is silent: it stops what runs, and an idle session hears nothing. A rewind tells the
  loop to read that session's history again. `delete` writes one memo file per session it removes —
  the session and every child under it — into the memo folder before the rows go.
- A refusal the user can actually cause is said in the session's own stream — one `stream notice`
  frame — instead of only in the answer the front end may drop. Today that is `delete` on a busy
  session and `cwd` on a path that does not exist or is not a workspace.
- `profile` is an argument of the write `init` and never a door of its own: a session's profile is
  fixed when it is created.
- A child session takes no messages: `submit` and `compact` answer `permission_denied` for one.
- The configuration is a startup snapshot. `call.config` answers the same values the process runs
  on; there is no door that re-reads the files, because the loop, the tool layer and the limits are
  built once.

### Door arguments

| kind | door | payload | answer |
|---|---|---|---|
| `register` | `hello` | `viewer` | an empty `opened` frame |
| `register` | `bye` | — | nothing |
| `call` | `init` | `session` | `structure` (`turn` + `ranges` per round), `blocks` (newest 32, whole rows), `todo` (the list, or null) |
| `call` | `blocks` | `session`, `ranges` | `blocks`, whole rows, in request order |
| `call` | `meta` | `session` | the session row |
| `call` | `usage` | `session` | session totals, `cache_hit_rate`, `cny`, `usd`, `context`, `estimated`, `baseline_prompt`, and `groups` by `provider:model` |
| `call` | `todo` | `session` | the replayed list, `[{"text", "done"}]` |
| `call` | `memo` | `session` (current only), `op`, `scope`, `name` | `names`, or `content`, or `history` |
| `call` | `children` | `session` | `rows`, each a child session's `id` and `anchor` |
| `call` | `config` | `key` | the value; a bare `models.model.<address>` answers the whole entry |
| `call` | `pools` | — | `models`, `profiles` |
| `call` | `sessions` | `under` (optional directory) | `rows`: main sessions, newest first, each carrying `first_user`, `bytes`, `last_ts`, `memos` |
| `post` | `init` | `cwd`, `name`, `model`, `profile` | `session` |
| `post` | `name` | `session`, `name` | — |
| `post` | `cwd` | `session`, `path` | `cwd` (resolved) |
| `post` | `switch` | `session`, `model` | — |
| `post` | `delete` | `session` | — |
| `post` | `interrupt` | `session` | — |
| `post` | `rewind` | `session`, `turn`, `block` | `cut` |
| `post` | `default_model` | `model` | `path` of the file written |
| `post` | `mailbox` | `session`, `text`, `prefix`, `suffix` (all but `session` optional) | `written`: whether a body was stored |
| `subscribe` | `watch` | `session`, or `null` to stop following | — |
| `subscribe` | `submit` | `session`, `text`, `prefix`, `suffix` | `turn` |
| `subscribe` | `compact` | `session`, `retention` (`{"chars"}` or `{"blocks"}`), `focus` | `turn` |

A `sessions` row is the session's own columns plus four the picker draws: `first_user` (the
first user block's body), `bytes` (the UTF-8 size of every block body), `last_ts` (the time of
the session's last block), and `memos` (how many distinct memo names the session holds).

`model` is `{"provider", "model"}` everywhere it appears. `usage`'s `groups` carry `address`,
`provider`, `model`, `rounds`, four token counts, `share`, and `price` (`input`, `output`,
`cache_read`, `cache_write`, `total`, `currency`, `cny`, `usd`); a model with no price answers
`null`. A block row is the `blocks` table's columns under their logical names.

### The session stream

Three sources feed one dispatcher:

```text
request half  --(Sender<Event>)-->  relay  -->  frames on every connection watching the session
storage       --(Structure)------->  relay
api's own write doors ------------->  relay
```

| event | `kind` | `name` | payload |
|---|---|---|---|
| write signal | `stream` | `signal` | the fields that write changed |
| slot taken | `stream` | `signal` | `signal: mailbox` |
| live output | `stream` | the door that opened the turn | `session`, `turn`, `kind`, the piece |
| notice | `stream` | `notice` | `session`, `level`, `content` |
| round structure | `stream` | the door that opened the turn | `session`, `turn`, `ranges` |

- `kind` in a live payload is one of `text`, `thinking`, `arguments`, `boundary`, `tool_start`,
  `tool_progress`, `tool_end`, `wake`. A `wake` piece is one job's reminder and keeps its own block.
- `text`, `thinking` and `arguments` carry `usage`: the bill of the request that just ended, on the
  one piece the decoder wrote it onto; `null` on every other piece.
- A write signal names the field its door changed: `cwd` with the landed path, `model` with the
  address, `name` with the new name. `delete` and `rewind` say so in `signal` and carry their own
  fields.
- A write signal skips the viewer that issued it. The slot-taken signal, live output, notices and
  structure go to everyone watching.
- The turn a live frame carries is recorded when `submit` or `compact` opens it. A turn the slot
  opened is announced by the loop on the same channel and is recorded under the viewer that wrote
  the slot, named `mailbox`; the same moment puts `signal: mailbox` on the stream. A turn a job
  woke names `wake`.
- The buffer belongs to the session, holds everything this turn said, and is handed to a connection
  that starts watching mid-turn. It is emptied when the round structure goes out, and dropped when
  the last watcher leaves.

### The writer

A session is idle or busy. Idle: whoever submits first opens the turn and becomes the writer.
Busy: the writer is fixed to the viewer that opened the turn; any other viewer's write door answers
`permission_denied`. The slot is cleared when the round structure arrives, not when the connection
goes away.

### Errors

`unknown_name`, `bad_payload`, `busy`, `permission_denied`, `internal`, all from
`ipc::contract::ErrorCode`. A `call`, `post` or `subscribe` frame before `hello` answers
`permission_denied`.

## 2. Design intent

- **The api is a postman.** It decodes a frame, names a door, and encodes the answer. Blocks,
  rounds, archives and tool results are the other halves' vocabulary, carried through untouched.
- **One stream per session, not per connection.** A second front end on the same session sees the
  same turn; the buffer is what makes joining mid-turn possible.
- **The writer is a turn, not a role.** It exists while a turn runs and only then, so it needs no
  takeover, no demotion, and no bookkeeping across turns.
- **A booking follows the watchers.** The loop holds a session's runtime — handle, threads, worker
  child, tabs — only while a connection follows it. The last watcher leaving unbooks it, and the
  next message books it again from its own rows, so nothing accumulates across a long server life.
- **The archive is written once.** A session's system prompt and tool manuals are what the front end
  chose when it opened the session; a later model switch changes the model, not the identity. An
  archive is identified by its system prompt, its tools, its provider and its model; an identical
  one is reused, and a child's is its own.
- **Config is read, never parsed, here.** `Config` owns the files; this domain asks for values and
  keeps no path.

## 3. Unresolved

None inside this domain. `ui::session` speaks these frames; the terminal front end still has to be
brought onto its own exit.

## 4. Config fields

| key | use |
|---|---|
| `config.server.api.defaultModel` | the model a session takes when `init` names none |
| `config.server.request.streaming` | `immediate` or `buffered`, handed to the request half |
| `config.server.request.compact.retainTail` | a compaction's default tail |
| `config.server.agent.loop.maxToolCalls` | the turn's tool-round fence; `false` turns it off |
| `config.server.agent.loop.scrooge` | true refuses a mid-session model change |
| `config.server.agent.loop.tools.memoFolder` | where `delete` writes the memo export; empty = the platform's download folder |
| `config.server.storage.batchTools` | unused: the writer commits one submission at a time |
| `config.ui.tui.fx.usdCny` | the display rate `usage` quotes in |
| `config.ui.tui.fx.currency` | `CNY` or `USD`: the side of the rate a front end counts in |
| `models.*` | the model table: caps, prices, the default model |

Every model must declare `maxOutputTokens`; a table without it is not a legal configuration and the
process refuses to start.
