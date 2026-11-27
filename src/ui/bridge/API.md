# bridge API

Everything a front end may say through the door, and everything it hears back. The door itself
carries frames unchanged; the names below are the server's, and this file is the copy a person
writing an adapter reads. `src/server/api/README.md` is where they are designed — when the two
disagree, that one is right.

## 1. The door

One port, one address, no path. The method says what is being asked for.

1. `GET /<file>` asks for one file of the webui directory; `/` is the page.
2. `POST /` asks for one frame, and answers with it.

A `POST` names itself in the `X-Bridge-Adapter` head. The name must be an adapter in
`adapters.yml`; anything else is refused with `403`. The body is one frame:

```json
{"kind": "call", "name": "init", "payload": {"session": 3}}
```

`payload` is always an object, possibly empty. There are four kinds a front end may send:

1. `call` reads and changes nothing; the answer is the read's payload.
2. `post` writes; the answer is `{}`, or the write's own answer.
3. `subscribe` asks for a stream or opens a turn; the answer is a stream, or the turn's reply.
4. `host` is the door's own, one directory's names; the answer is the listing.

Nothing is remembered between requests. A front end that wants to know the answer to a question
sends it again.

## 2. Following a conversation

`subscribe` / `watch` with a `session` does not answer with a reply: it turns the connection into
that conversation's stream, one frame per line, and keeps it open until the peer leaves.

```
data: {"kind":"stream","name":"submit","payload":{...}}
```

Three things follow, and they matter more than they look.

1. **A held session lives while somebody follows it.** The server keeps a session's runtime — its
   round, its worker, its jobs, its browser tabs — only while a connection follows it. Stop
   following, and it is released. This is why an adapter that hosts several conversations holds
   several connections rather than one.
2. **A conversation has one writer at a time.** Whoever opens a turn owns it; another front end's
   write answers `writer_required`. A session whose last watcher left keeps its turn but loses its
   name, so the next front end to write claims it.
3. **A connection follows one conversation.** `watch` switches which; `watch` with `session: null`
   stops following.

## 3. What the stream carries

A frame's `name` says what it is, and its `payload` says what it carries:

1. `notice` carries `session` and `text`: one line the server has to say.
2. `signal` carries the write's own fields, plus `signal` naming it.
3. The name that opened the turn — `submit`, `mailbox`, `wake`, `compact` — carries `session`,
   `turn`, `kind`, and the piece.
4. The same name carries `session`, `turn`, `ranges`, `compact` once the round is over.

The last is the receipt. It is the only frame that says a round ended: when its `payload` has
`ranges`, the round is closed and everything before it belonged to that round. A front end that
missed the beginning of a round is given the whole round from its first frame the moment it
starts following, so a receipt always closes what the front end has actually seen.

`kind` in a live piece is one of:

1. `text` is the assistant's words; blocks of it add up to one block.
2. `thinking` is the model's reasoning, when the model exposes it.
3. `arguments` is one tool call's arguments, as they are written.
4. `boundary` says a block just ended — nothing follows it in that block.
5. `tool_start` carries `call_id`, `name`, `args`, `intent`.
6. `tool_progress` carries `call_id` and `text`.
7. `tool_end` carries `call_id`, `ok` (one outcome per item), `result`, `memo`, `duration_ms`,
   `truncated`.
8. `wake` is a job's reminder, which keeps a block of its own.

A tool's name arrives on `tool_start` and nowhere else; pair it with `tool_end` by `call_id`.
`text` is the unit that opens and closes: `boundary` ends it, so a block of prose never spans one.

Signals name a write that landed: `signal: "model"` with the address, `"cwd"` with the path,
`"name"`, `"effort"`, `"rewind"` with `turn`/`block`/`cut`, `"user"` and `"mailbox"` with the
opening words. A signal skips the front end that issued the write; live output, notices and the
receipt go to everyone watching.

## 4. Reads

1. `init` takes `session` and answers `structure` (each round's `turn`, `ranges`, `compact`),
   `blocks` (the newest 32 whole rows), `todo` and `effort`.
2. `blocks` takes `session` and `ranges` (`[[lo, hi]]`) and answers `session` and `blocks`.
3. `meta` takes `session` and answers the session row.
4. `usage` takes `session` and answers the bill: tokens, `cache_hit_rate`, `cny`, `usd`, `context`,
   `estimated`, `baseline_prompt`, `groups`, and the subagents' `subagent_price`,
   `subagent_groups`.
5. `todo` takes `session` and answers `[{"text", "done"}]`.
6. `memo` takes `session`, `op`, `scope` and `name`, and answers `names`, `content` or `history`, by
   `op`.
7. `children` takes `session` and answers `session` and `rows`: each child's `id`, `name`, `cwd`,
   `anchor`, `rounds`, `bytes`, `last_ts`, `first_user`, `running`.
8. `config` takes `key` and answers the value; `models.model.<address>` answers that whole entry.
9. `pools` takes `cwd` and `session`, both optional, and answers `models` and `profiles`.
10. `sessions` takes `under`, an optional directory, and answers `rows`, newest first.

A `sessions` row carries the session's own columns plus `first_user`, `bytes`, `last_ts` and
`memos`. A `pools` model carries `provider`, `id`, `address`, `name`, `context_window`,
`max_output_tokens`, `currency`.

## 5. Writes

1. `init` takes `cwd`, `name`, `model`, `profile`, `effort` and `fallback`, and answers
   `{"session"}`, plus `cwd` when `fallback` moved the directory.
2. `delete` takes `session` and answers `{}`; that session's stream closes.
3. `memo` takes `session`, `op`, `scope`, `name` and `content`, and answers `{}`.
4. `default_model` takes `model` and answers `{}`.
5. `interrupt` takes `session` and answers `{}`.
6. `mailbox` takes `session`, `text`, `prefix` and `suffix`, and answers `{}`.
7. `meta` takes `name` and `content`, and answers `{}`; the result is a signal on the stream.

`init` without `fallback` takes the directory as given and refuses one that does not exist: the
user picked it. With `fallback`, the front end only carried the path over from where it started,
and a directory the session may not stand in is replaced by the temp directory.

`mailbox` is a conversation's one waiting message. `text` writes it whole over what was there; an
empty `text` with a `prefix` or `suffix` still writes; neither clears it. The loop takes it at its
next blocking point and opens a turn named `mailbox`.

`meta` is the one nested write: its `name` picks which field changes, its `content` holds that
write's arguments. Each of its five names takes `session` and one more field, and comes back as a
signal:

1. `name` takes `name`; the signal is `{"signal": "name", "name"}`.
2. `cwd` takes `path`; the signal is `{"signal": "cwd", "cwd"}`.
3. `switch` takes `model`; the signal is `{"signal": "model", "model"}`.
4. `effort` takes `effort`; the signal is `{"signal": "effort", "effort"}`.
5. `rewind` takes `turn` and `block`; the signal is
   `{"signal": "rewind", "turn", "block", "cut"}`.

`model` is `{"provider", "model"}` everywhere it appears.

## 6. Opening a turn

1. `submit` takes `session`, `text`, `prefix` and `suffix`, and answers `turn`.
2. `compact` takes `session`, `retention` (`{"chars"}` or `{"blocks"}`) and `focus`, and answers
   `turn`.

Both answer `busy` when the conversation is already in a turn. Nothing is queued here: a message
that must wait goes to `mailbox` instead. A child session takes no messages and answers
`child_message`.

`prefix` and `suffix` are carried in front of and behind the user's words. They are how a front end
tells the model what it cannot otherwise know — a chat has no clock.

## 7. The door's own frame

`host` / `list` takes `path` and `cwd`, and answers `{"dir", "entries": [{"name", "dir"}]}`.

`path` is a half-typed directory: its own head is resolved against `cwd`, `~` is home, and one
name per entry comes back with a flag saying whether it is a directory. It never reaches the
server.

## 8. Errors

A refusal answers with the request's own `kind` and `name`, and this payload:

```json
{"error": {"kind": "error", "key": "payload_key_missing", "args": {"key": "session"}}}
```

A refusal the door itself made carries `code` and `message` instead, with an HTTP status:

```json
{"error": {"code": "unknown_adapter", "message": "no adapter of that name in adapters.yml"}}
```

A server error may carry `text` in place of `key`, which is a line meant to be shown as it stands.
Keyed ones are meant to be read, not shown: `key` says what happened and `args` says about what.

## 9. Filtering, per adapter

`adapters.yml` decides what each adapter is fed: `thinking` and `tools` switch whole categories
off, and `chunk` cuts one `text` piece to a length. Only categories are separated. Whether a tool
call is paired with its result, and how the words are laid out, is the receiver's business.

## 10. Keeping this file true

Four things here have a single owner elsewhere, and a change to one is a change to this file:

1. The four kinds, and `host` among them — the door's own switch in `http/serve.rs`.
2. The name tables in §4, §5, §6 and §7 — `src/server/api/route.rs`.
3. The live `kind`s in §3 — the relay's payloads, and `adapter/events.rs`'s two lists.
4. The receipt — the frame carrying `ranges`, which `manager/held.rs` also reads to know a round
   ended.