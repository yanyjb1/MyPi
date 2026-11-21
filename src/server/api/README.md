# api

The backend's only front-facing surface. One connection table, one stream per session, and one
name per action. Nothing here knows what a block means; it moves rows and frames.

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
| `call` | front end | a read name | that name's arguments |
| `post` | front end | `meta` / `init` / `delete` / `memo` / `default_model` / `interrupt` / `mailbox` | that name's arguments |
| `subscribe` | front end | `watch` / `submit` / `compact` | that name's arguments |
| `stream` | back end | `signal` / `notice` / the turn's name | the pushed row |

A reply carries the request's `kind` and `name`, so a front end matches an answer to its own
request without a counter. A failure is a reply whose payload is `{"error": {"code", "message"}}`.
Every `post` answers `{}` except `init`, which answers `{"session"}`.

`post delete` is its own name: it answers `{}` and closes every removed session's stream, with no signal.

`post meta` is the write path for session metadata. Its payload is `{"name", "content"}`: `name`
picks the write (`name`, `cwd`, `switch`, `effort`, `rewind`) and `content` holds that
write's arguments. The result leaves the session stream as `stream signal`, whose payload is the
write's own fields plus `signal` naming it. The issuer is skipped.

### Names

Read: `init`, `blocks`, `meta`, `usage`, `todo`, `memo`, `children`, `config`, `pools`, `sessions`.
Write: `meta` (`name`, `cwd`, `switch`, `effort`, `rewind`), `init`, `delete`, `memo`,
`default_model`, `interrupt`, `mailbox`.
Subscribe: `watch`, `submit`, `compact`.

  child's session row.
- `init` lands the directory it was handed. `fallback` says the front end only carried it over from
  where it was started: a directory a session may not stand in then becomes the temp directory, and
  the answer names the landed path. Without `fallback` the path must exist and is taken as given, so
  a user-picked directory is never moved behind their back.
- `init` also takes `effort`: the rung a draft picked before its row existed. Without it the session
  opens on `auto`, the one rung every model shares.
- `init` (call), `blocks`, `meta`, `memo`, `children`, `sessions` read through `storage::Reader`
  only. The api holds no SQL and decodes no column.
- `todo` answers the whole list `Loop::todo` replayed from the session's own `todo` calls.
- `usage` groups a session's turns by the `provider` and `model` each turn recorded, names each
  group by its `provider:model` address, orders the groups by their token share, and quotes each
  one with `models.yml`'s price in its own currency plus both display currencies. The bill is the
  stored stamps and nothing else: a lap counts once, when the block carrying its usage lands. A
  running turn therefore shows up one lap late and settles the moment it closes, so the answer only
  ever grows inside a session. One answer serves the cost pill, the usage panel and the gauge.

- `usage` also bills the session's subagents: one pool, every session this conversation spawned
  (its child rows hang on blocks the session's own ranges hold), each child billed at the model its
  turns recorded, at that model's price. The answer carries `subagent_price` (`cny`/`usd`, `null`
  when no priced model ran) and `subagent_groups` in the same shape as `groups`. Children cannot
  spawn children, so one level of enumeration is exact; rewind sweeps dead children with the
  blocks they hang on, so the pool follows the visible history like the session's own bill.
- `usage` also answers where the context stands: the prompt of the newest stored stamp. A
  compaction is the exception: the context is then the summary plus whatever tail it kept, so
  `usage` estimates it instead of reporting the pre-compaction prompt. The compaction round
  measured what it replaced; the newest measurement before it measured the context as it stood.
  Removing the first from the second leaves the tail, which the summary's own completion and the
  overhead every request carries (the baseline) are then added to. The answer says `estimated`.
  `baseline_prompt` is that floor: the smallest first-turn prompt among the sessions that share
  this request archive, which is what a front end shows before the first request lands.
- `submit` and `compact` open a turn; a session already in a turn answers `busy`. Nothing is queued
  by the api: a front end that wants a sentence to wait writes it to `mailbox` instead.
- `mailbox` is the session's one pending message. `text` present and non-empty, or an empty `text`
  with a `prefix` or `suffix`, writes it whole over whatever was there; no `text` and no prefix
  clears it. The loop takes it at a blocking point and opens a turn named `mailbox`, exactly as a
  `submit` would; taking it clears it and puts `signal: mailbox` on the stream, carrying the words
  for every watcher but the writer.
- `effort` sets how hard the session's next laps ask the model to think. It is the one write
  that changes no row: the rung lives on the session's request config and dies with the booking. A
  model that declares no rung answers `effort_unsupported` rather than accepting a value it could
  never send, and a rung the model does not declare answers `effort_not_declared`, with the
  declared ones named.
  A model switch drops the rung back to `auto`, because the rung belonged to the model that declared
  it.
  `interrupt` is silent: it stops what runs, and an idle session hears nothing. A rewind tells the
  loop to read that session's history again. `delete` writes one memo file per session it removes —
  the session and every child under it — into the memo folder before the rows go.
- A refusal the user can actually cause is said in the session's own stream — one `stream notice`
  frame — instead of only in the answer the front end may drop. Today that is `delete` on a busy
  session. A refused `cwd` answers in its reply alone: it is the one write whose reader is waiting
  on that very answer.
- `profile` is an argument of the write `init` and never a name of its own: a session's profile is
  fixed when it is created.
- A child session takes no messages: `submit` and `compact` answer `child_message` for one.
- The configuration is a startup snapshot. `call.config` answers the same values the process runs
  on; there is no name that re-reads the files, because the loop, the tool layer and the limits are
  built once.

### Name arguments

| kind | name | payload | answer |
|---|---|---|---|
| `register` | `hello` | `viewer` | `{"notices"}` |
| `register` | `bye` | — | nothing |
| `call` | `init` | `session` | `structure` (`turn` + `ranges` + `compact` per round, the whole list, borrowed rounds left out), `blocks` (newest 32, whole rows), `todo` (the list, or null), `effort` (the rung the session runs with now) |
| `call` | `blocks` | `session`, `ranges` | `session`, and `blocks`: whole rows, in request order |
| `call` | `meta` | `session` | the session row |
| `call` | `usage` | `session` | session totals, `cache_hit_rate`, `cny`, `usd`, `context`, `estimated`, `baseline_prompt`, `groups` by `provider:model`, and the subagents' `subagent_price` + `subagent_groups` |
| `call` | `todo` | `session` | the replayed list, `[{"text", "done"}]` |
| `call` | `memo` | `session` (current only), `op`, `scope`, `name` | `names`, or `content`, or `history` |
| `call` | `children` | `session` | `session`, and `rows`: each child's `id`, `name`, `cwd`, `anchor`, `rounds`, `bytes`, `last_ts`, `first_user` and `running` |
| `call` | `config` | `key` | the value; a bare `models.model.<address>` answers the whole entry |
| `call` | `pools` | — | `models`, `profiles` |
| `call` | `sessions` | `under` (optional directory) | `rows`: main sessions, newest first, each carrying `first_user`, `bytes`, `last_ts`, `memos` |
| `post` | `meta` | `name`, `content` | `{}`; the result is a `stream signal` |
| `post` | `init` | `cwd`, `name`, `model`, `profile`, `effort`, `fallback` (optional) | `{"session"}`, plus `cwd` when `fallback` moved the directory |
| `post` | `delete` | `session` | `{}`; the session's stream closes |
| `post` | `memo` | `session` (current only), `op`, `scope`, `name`, `content` | `{}` |
| `post` | `default_model` | `model` | `{}` |
| `post` | `interrupt` | `session` | `{}` |
| `post` | `mailbox` | `session`, `text`, `prefix`, `suffix` (all but `session` optional) | `{}` |
| `subscribe` | `watch` | `session`, or `null` to stop following | — |
| `subscribe` | `submit` | `session`, `text`, `prefix`, `suffix` | `turn` |
| `subscribe` | `compact` | `session`, `retention` (`{"chars"}` or `{"blocks"}`), `focus` | `turn` |

A `post meta` payload's `name` and `content`:

| name | content | signal |
|---|---|---|
| `name` | `session`, `name` | `{"signal": "name", "name": ...}` |
| `cwd` | `session`, `path` | `{"signal": "cwd", "cwd": ...}` |
| `switch` | `session`, `model` | `{"signal": "model", "model": ...}` |
| `effort` | `session`, `effort` | `{"signal": "effort", "effort": ...}` |
| `rewind` | `session`, `turn`, `block` | `{"signal": "rewind", "turn", "block", "cut"}` |

A `sessions` row is the session's own columns plus four the picker draws: `first_user` (the
first non-empty user body), `bytes` (the UTF-8 size of every block body), `last_ts` (the time of
the session's last block), and `memos` (how many distinct memo names the session holds).

`model` is `{"provider", "model"}` everywhere it appears. `usage`'s `groups` carry `address`,
`provider`, `model`, `rounds`, four token counts, `share`, and `price` (`input`, `output`,
`cache_read`, `cache_write`, `total`, `currency`, `cny`, `usd`); a model with no price answers
`null`. A block row is the `blocks` table's columns under their logical names.

### The session stream

Two sources feed one dispatcher, one queue:

```text
request half  --(Sender<Event>)-->  relay  -->  frames on every connection watching the session
storage       --(ranges, by the loop's settle)-->  the same Sender<Event>
api's own writes ------------------->  relay
```

The loop settles a turn through `pen::turn::settle`, which hands back the closed ranges and whether
the round carries the compaction mark; the loop sends both as `Element::Structure` on the same
sender as everything else that turn said. A front end therefore hears the receipt before the next
turn's first frame, always — not usually.

| event | `kind` | `name` | payload |
|---|---|---|---|
| write signal | `stream` | `signal` | the write's fields plus `signal` |
| live output | `stream` | the name that opened the turn | `session`, `turn`, `kind`, the piece |
| notice | `stream` | `notice` | `session`, `text` |
| slot taken | `stream` | `signal` | `signal: mailbox`, and `text` for every watcher but the writer |
| turn opened | `stream` | `signal` | `signal: user`, `text` |

- `kind` in a live payload is one of `text`, `thinking`, `arguments`, `boundary`, `tool_start`,
  `tool_progress`, `tool_end`, `wake`. A `wake` piece is one job's reminder and keeps its own block.
- `tool_end` carries `ok` as the array of item outcomes — one entry per item the call's arguments
  named, `{ok, error}` — plus `result`, `memo`, `duration_ms` and `truncated`. The stored row's `ok`
  column holds the same array.
- `text`, `thinking` and `arguments` carry `usage`: the bill of the request that just ended, on the
  one piece the decoder wrote it onto; `null` on every other piece.
- A write signal carries `signal` naming the write, plus its own fields: `cwd` with the landed
  path, `model` with the address, `name` with the new name, `effort` with the rung, `rewind` with
  `turn`, `block` and `cut`. `delete` sends no signal: it closes the stream instead.
- A write signal skips the viewer that issued it. The slot-taken signal, the turn-opened signal,
  live output, notices and structure go to everyone watching.

- A turn's opening words reach the other front ends as `signal: user`; a letter taken into a running
  turn reaches them as `signal: mailbox`. Both name the words in `text` and both skip the writer,
  which echoes its own message already. The writer's own copy of the slot signal carries no `text`:
  that absence is how a front end knows the words are its own to echo. A turn opened from the slot
  on an idle session names `mailbox`; a letter taken into a running turn keeps that turn's opener.
  A turn a job woke names `wake`.
- The buffer belongs to the session, holds everything this turn said, and is handed to a connection
  that starts watching mid-turn. It is emptied when the round structure goes out, and dropped when
  the last watcher leaves.

### The writer

A session is idle or busy. Idle: whoever submits first opens the turn and becomes the writer.
Busy: the writer is fixed to the viewer that opened the turn; any other viewer's write answers
`writer_required`. The slot is cleared when the round structure arrives, not when the connection
goes away.

### Errors

An error frame carries `ipc::contract::Text`: a `kind` plus a machine `key` and optional `args`;
an upstream body carries `text` instead. A `call`, `post` or `subscribe` frame before `hello`
answers the key `hello_required`.

## 2. Design intent

- **The api is a postman.** It decodes a frame, names the write or read, and encodes the answer. Blocks,
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
