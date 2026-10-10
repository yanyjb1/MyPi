# api

The surface a front end talks to, and the server's own reuse face. Nothing else under `server/` is a
front end's business.

## 1. The design

### Three doors

| door | what it is | what comes back |
|---|---|---|
| `call` | one question, one answer | a `Reply`, at once |
| `post` | one change of session state | a `PostDone` |
| `subscribe` | open a stretch | a channel, or a refusal |

**`call`** — `call::answer(cfg, call)`. Eight questions: the usage bill, the resume picker's rows,
three viewport reads (the first frame, older, newer), the todo list, a session's metadata, the two
completion pools. The front end reads no database and does no arithmetic; it gets a finished answer.

**`post`** — `post::apply(cfg, post)`. Nine changes: `/name`, `/cdp`, `/switch`, `/model`,
`/profile`, creating a session, deleting one, an interrupt, a rewind. A change answers nothing — the
value it changed is the caller's own, and no error back means it landed. Creating a session is the
one exception: without the id the front end cannot open a round, so that one answers with it.

An interrupt is fire-and-forget: the flag being set is all it promises, and the truth arrives as the
`StreamItem::Ended` that closes the running round's stream. A rewind is one-shot too: it cuts the
list and sweeps what nothing references, and the front end works out what survived from the window it
already holds.

**`subscribe`** — three entries. One watches a session, two open a stretch on it:

| entry | does | answers |
|---|---|---|
| `subscribe::watch(cfg, viewer, session)` | claims the session's stream for one viewer | that stream, which carries every piece about the session until the viewer stops watching |
| `subscribe::submit(cfg, session, prefix, text, suffix)` | opens one round | `Opened::Started`, or `Opened::Busy` when the session is already mid-round |
| `subscribe::compact(cfg, session, retention, focus)` | opens one compaction | the same two, `Busy` also when there is nothing to compact |

A round's content does **not** come back from `submit`: one session has one stream per viewer, and a
surface watches *before* it asks. That is what lets two surfaces watch one session without one of
them stealing the other's pieces. A stretch ends at `StreamItem::Ended`, which names the round it
closed; whoever stopped it, that is the end.

Watching stops with `Post::Unwatch` (`viewer` + `session`, no answer). A front end makes its own id
up at startup — `Viewer` — and carries it on every claim; the server never invents one.

What the doors send back is public: `Reply`, `PostDone`, `StreamItem`, `Opened`. Everything else is
this layer's own business.

### The api's state

Nothing outside this layer holds a handle to it. The three doors reach it through one private
`state(cfg)`, which opens the database the first time it is asked and keeps it for the life of the
process. The file is `history.db` under the platform's data directory — the XDG one
(`$XDG_DATA_HOME/mypi`, else `~/.local/share/mypi`).

Inside it: the writer, one `Live` per session, the set of running rounds, the relay, and the config.

### The shapes

`types.rs` carries the doors' vocabulary: `Call`, `Post`, `Subscribe`, `Reply`, `PostDone`,
`Viewer`, `ModelId`, `UsageLang`, `CandidatePools`. Every argument is a **parsed value** — a surface
parses its own input and hands typed pieces over; no raw command line crosses this file. `Reply` may
name the modules that define its payloads.

`reply/stream.rs` carries what a viewer sees on its stream: `StreamItem`, whose `Ended` names the
round that just closed. `Relayed` is one piece as it leaves storage's port, with the session it
belongs to.

`Opened` (in `subscribe`) is the verdict a stretch door answers with: started, or busy.

### A round

1. A surface calls `watch` for the session it wants to see (once, when it opens it).
2. It calls `submit` with the session id and the message already split into prefix, body and suffix.
3. The session is claimed. Already running → `Busy`, and nothing else happens.
4. The session's `Live` is borrowed from the session table (built from the session row, the config and the session's own transcript, if this is the first round).
5. The user's message is pushed to the writer and **lands at once** — the round row and the user's block go in together, and the answer carries the round id.
6. A thread runs `agent::loop::run` for that round. The loop cuts the reply into blocks, hands them to storage, runs the tools, and settles the round when it stops. Every piece reaches the watchers on the session's stream.
7. The claim is released and the `Live` goes back — or is let go, when nobody is watching any more. Everything that landed is in storage, so a session touched again builds its state from its transcript.

A compaction takes the same shape: claim the session, plan the cut from the transcript, and run the
summarization on a thread. The summary lands as a round of its own, and the kept stretch gets one
borrowed index per whole round. It is a round like any other — it just runs no tools — so storage
relays its end when it lands. A summarization that never lands settles its round here instead, with
no reason recorded, so the stretch still ends.

### The session table

One `Live` per session: the provider connection, the working directory **in memory**, the tool
roster, the todo list, the stop flag, the artifact handle (what a `#<ref>` materializes into a temp
file), and the settings in effect. It is built on demand and lives between rounds.

**When the last viewer leaves and the last round ends, the `Live` is let go.** Two questions decide
it — is anybody watching (the relay's list), is a round running (the running set) — and both the
last unwatch and the round's own end ask them. Letting it go drops the provider connection and the
subprocess behind it, and the temp files that session's artifacts were written into; nothing else,
because everything that matters is in storage and the next round builds the state again. What the
slot remembers besides the `Live` — the profile chosen mid-session, the compaction instruction of
the profile in effect — stays.

Two things a session row cannot carry sit beside it: the profile chosen mid-session (it lands at the
next of two moments), and the compaction instruction of the profile in effect.

The working directory is the one piece of state that is not in the database at all: the model's `cd`
moves it, `/cdp` resolves `~` and relative paths against it, and nothing writes it to a row. A front
end reads it through `Call::Meta`.

### The relay, and the text that streams

Storage relays to exactly one port — the api's. Every piece names its session. The state keeps the
subscribers **per session, keyed by viewer** and hands each piece to every viewer of that session; a
stream that nobody holds any more is dropped on the spot, which is also how the state knows when a
session has no viewers left. The api's own pieces go through the same fan-out: the reply's live
increments, and a tool's running output. (A compaction's end is not one of them: it lands in storage,
so storage relays it.)

The loop's outlet hands the reply's increments to the writer as a block that is **still being
written**. The writer relays every piece the moment it arrives and holds it back from the disk; the
request side's cut closes the block, and that cut is the version that lands. A front end sees the
text as it streams, and the transcript gets one whole block per message.

A tool's running output is the one piece that never becomes a block: it goes straight to the same
fan-out, because there is no block for it to belong to.

### Reading

A reader opens its own connection (`Reader::open`), writes its own SQL, decodes rows with
`storage::decode`'s column lists and decoders, and closes the connection. `call/*` and `post/*` do
this per request; the state does it when it needs the session row.

### Invariants

1. One session runs one round at a time. The claim is the lock; storage does not know about it.
2. The user's message lands before the loop starts. The loop reads it back out of the list.
3. A round's content is landed by the request side, never by the api's preview.
4. Every query is written where it is asked, and no layer above storage spells out a column list.
5. `Post::Init` is the only change that answers with a body.
6. One session has one stream per viewer, and a stream outlives any single round: a round's end
   names the round it closed instead of closing the stream.
7. A session's `Live` is let go only when nobody watches it and nothing runs in it.

## 2. Design intent

**Three doors, because the three ways of talking are genuinely different.** A question has an answer;
a change does not (the value it changed is the caller's own, and no error means it landed); a stretch
streams, so it answers with a channel and the content arrives on it.

**No handle, because a front end should hold no server state.** Opening a database, keeping one
`Live` per session, remembering which rounds are running and routing storage's relay are not a front
end's business — and neither is remembering to build them. The state builds itself on first use, and
a surface names only the three doors.

**Interrupt and rewind are changes, not subscriptions.** Neither streams anything of its own: an
interrupt sets a flag and a rewind cuts a list. Their receipts are either slow (the round's own end)
or uninteresting (a cut the front end can work out from its own window), so they answer nothing at
all rather than inventing a second way to say "done".

**A session id is mandatory.** A front end creates the session first (`Post::Init`) and opens rounds
with the id it got back; there is no path that creates one on the way in.

**`/profile` has two moments, because the profile is a prefix.** Swapping the system prompt and the
tool roster mid-round would change the bytes a request is built from while it runs. It lands before a
new session's first message, and before the first message after a compaction — the two points where
the prefix is rebuilt anyway. A profile chosen in between waits in the session table.

**The writer separates forwarding from writing.** A reply's text is handed in while it is still
being written: relayed at once, never landed. Only a finished block is written, and the request
side's cut is what finishes it. That keeps one rule in one place — a surface sees text the moment it
exists, the disk sees whole blocks — and the same channel carries both, so no second path to the
surface is needed.

**Reading opens its own connection, because reads must not wait for the writer.** A reader is never
locked out by a batch landing. It also keeps the api free of a shared connection: the writer's queue
is the only shared thing.

**One `Live` per session, built on demand, because a round needs state that outlives it.** The
connection, the roster and the in-memory directory are expensive to rebuild and wrong to share. A
round borrows its session's `Live` and gives it back; two sessions never touch the same one.

**`/switch` retargets the running session.** The row is the durable answer, and the state updates the
`Live`'s connection so the next round uses the new model without a restart.

## 3. Open questions

1. **Nothing calls these three doors yet.** The entry points reach the ipc layer directly, so
   `watch` / `submit` / `compact` have no caller outside this layer (第 8 条).
2. **Nothing renews the lists after a config reload.** The completion pools and the tool roster are
   snapshots taken when a session's state is built (第 20 条).
3. **The fork mode.** `rewind_non_prune` is a `todo!()`.
4. **The `Subscribe` shape has no caller.** The three entries take loose arguments, so nothing
   constructs `Subscribe`; the front end's own wave decides whether it stays.

## 4. Config

Read through `Config`; this layer opens no file.

| key | what it decides |
|---|---|
| `config.server.storage.batchTools` | handed to `Writer::open` — how many tool rounds land one batch |
| `config.server.agent.profile` | the profile a session starts under (the built-in `oh-my-pi` when absent) |
| `config.server.agent.loop.maxToolCalls` | the loop's fence, as `Live.settings.fence` |
| `config.server.agent.tools.*` | `bashTimeoutSecs`, the output limits, the browser package, `emptyReply` — the rest of `Live.settings` |
| `config.server.request.compact.retainTail` | the tail a compaction keeps when the user names no budget |
| `config.server.request.compact.instructionFile` | the compaction template, unless the profile carries its own |
| `config.server.api.defaultModel` | the model a session without one runs |
| `models.yml` | the provider, its endpoint, its key (`${ENV}` resolved), and the model's cost and `maxOutputTokens` |
| `config.tui.fx` | the currency folding the `/usage` bill applies to unit prices |
