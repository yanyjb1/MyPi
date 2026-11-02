# api

The backend's only front-facing surface. One connection table, one stream per session, and one
door per name. Nothing here knows what a block means; it moves rows and frames.

## 1. Design / Schema

### The connection table

Every front end opens one connection and says `hello` with a viewer id. That id is what the
session's writer slot is keyed by. A connection follows at most one session at a time; `watch`
switches which one, and the previous session keeps whatever is already in its buffer.

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
Write: `init`, `name`, `cwd`, `switch`, `delete`, `interrupt`, `rewind`, `default_model`.
Subscribe: `watch`, `submit`, `compact`.

- `init` (post) writes the `sessions` row and its `request_meta` archive in one submission, then
  books the session. The archive is written exactly once per session; the model may change later,
  the archive may not.
- `init` (call), `blocks`, `meta`, `memo`, `children`, `sessions` read through `storage::Reader`
  only. The api holds no SQL and decodes no column.
- `todo` answers the whole list `Loop::todo` replayed from the session's own `todo` calls.
- `usage` groups a session's turns by the `provider` and `model` each turn recorded, names each
  group by its `provider:model` address, orders the groups by their token share, and quotes each
  one with `models.yml`'s price in its own currency plus both display currencies.
- `submit` and `compact` open a turn; a session already in a turn answers `busy`.
- `interrupt`, `cwd`, `switch`, `delete`, `rewind` act on the running session or its rows.
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
| `call` | `init` | `session` | `structure` (`turn` + `ranges` per round), `blocks` (newest 64, whole rows), `todo` (the list, or null) |
| `call` | `blocks` | `session`, `ranges` | `blocks`, whole rows, in request order |
| `call` | `meta` | `session` | the session row |
| `call` | `usage` | `session` | session totals, `cache_hit_rate`, and `groups` by `provider:model` |
| `call` | `todo` | `session` | the replayed list, `[{"text", "done"}]` |
| `call` | `memo` | `session` (current only), `op`, `scope`, `name` | `names`, or `content`, or `history` |
| `call` | `children` | `session` | `rows`, each a child session's `id` and `anchor` |
| `call` | `config` | `key` | the value |
| `call` | `pools` | — | `models`, `profiles` |
| `call` | `sessions` | `under` (optional directory) | `rows`: main sessions, newest first, each carrying `first_user`, `bytes`, `last_ts`, `memos` |
| `post` | `init` | `cwd`, `name`, `model`, `profile` | `session` |
| `post` | `name` | `session`, `name` | — |
| `post` | `cwd` | `session`, `path` | — |
| `post` | `switch` | `session`, `model` | — |
| `post` | `delete` | `session` | — |
| `post` | `interrupt` | `session` | — |
| `post` | `rewind` | `session`, `turn`, `block` | `cut` |
| `post` | `default_model` | `model` | `path` of the file written |
| `subscribe` | `watch` | `session` | — |
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
| write change | `stream` | `change` | `session`, `action`, `by`, the changed row |
| live output | `stream` | the door that opened the turn | `session`, `turn`, `kind`, the piece |
| notice | `stream` | `notice` | `session`, `level`, `content` |
| round structure | `stream` | the door that opened the turn | `session`, `turn`, `ranges` |

- `kind` in a live payload is one of `text`, `thinking`, `arguments`, `boundary`, `tool_start`,
  `tool_progress`, `tool_end`.
- `action` in a change payload is `name`, `cwd`, `switch`, `rewind` or `delete`; the rest of the
  payload is that door's changed row.
- A change frame skips the viewer that issued it; live output, notices and structure go to
  everyone watching.
- The turn a live frame carries is recorded when `submit` or `compact` opens it, so the loop's
  events need no turn of their own. A turn a job woke names `wake`.
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
- **The archive is written once.** A session's system prompt and tool manuals are what the front end
  chose when it opened the session; a later model switch changes the model, not the identity.
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
| `config.server.storage.batchTools` | unused: the writer commits one submission at a time |
| `config.ui.tui.fx.usdCny` | the display rate `usage` quotes in |
| `models.*` | the model table: caps, prices, the default model |

Every model must declare `maxOutputTokens`; a table without it is not a legal configuration and the
process refuses to start.
