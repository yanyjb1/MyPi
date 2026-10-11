# session — the layer every mypi front end sits on

mypi is a terminal coding agent: one server process owns the database and the agent loop; front ends
talk to it over a socket. This directory is the layer a front end sits on: the connection, the
session's state, and the blocks waiting to be rendered. A front end reads a borrowed view of it and
draws.

## 1. Design / Schema

### A session is one viewer's view of one stored session

One session at a time: watching another means leaving the first. Several viewers may watch the same
session, each with a copy of this layer of its own; this layer knows nothing about that.

### Files

| file | holds |
|---|---|
| `mod.rs` | `Session` — the one entry point for every api call; `View` — the borrowed read model |
| `client.rs` | the connection, the request queue, the session's stream |
| `ingest.rs` | what the backend sends → state, window and structure |
| `state.rs` | the session's state |
| `window.rs` | the content cache: the landed window and the unlanded tail |
| `shape.rs` | the structure cache: which blocks each kept round manages |
| `command.rs` | a slash command's arguments → the call that carries them out |

### The entry point

```text
connect(cost, path, cwd, switches)   bye()
set_draft(...)   open_new()   attach(id)   detach()
submit(text)   interrupt()   compact(retention, focus?)   rewind(turn, block)
rename(name?)  set_cwd(path)  switch_model(id)  set_default_model(id)
set_profile(profile)  delete(id)
list_sessions(under?)   usage(lang)   todo()   pools()
note_view(top?, bottom?, pinned)   view()   text_of(block)
set_preload(n)   set_interrupted_text(text)   notice(text, level, life)
```

Two rules per method. A change lands locally first and joins one queue, which the frame loop sends
with `pump` once per frame. A read is a question, and its answer comes back through `ingest` like
everything else.

### The read model

`View` is borrowed: the session's id and flags, name and model, the working directory and whether
the model moved it, the notices, the task list, the picker's rows, the completion pools, the bill,
the landed blocks (`chain()` walks landed, tail and the optimistic row in display order as
`BlockRef`), the tail, the optimistic interrupt row, the kept structure, and a generation counter
that moves on a whole replacement.

### Two ways to read one block

`BlockRef` is one shape over a landed block (`Block`, which has an id) and one that has not landed
(`NewBlock`): kind, content, tool name, arguments, intent, the half sentence the model wrote,
whether it is the reply's first call, success, result, the extra material for a front end, duration,
the pairing key, usage, timestamp.

### Two caches

| cache | holds | budget |
|---|---|---|
| content (`window.rs`) | the landed window, ascending by id, plus the tail that has not landed | `config.ui.session.preloadCache` blocks around the view, evicted from the end farthest from the view |
| structure (`shape.rs`) | per kept round: its block ids and each block's content hash, newest first | 8 rounds |

The tail accumulates: an increment is appended to its block, a finished piece replaces it and
carries the hash that claims its id. A landed batch is appended by id, and a block already there
takes the landed version. `claim` matches a round's (id, hash) pairs against the window and the
tail: a finished tail block that matches lands with that id.

### One round

```text
submit → the answer carries the round id
increments → appended to the tail
finish → the block is replaced, its hash taken
a landed batch → appended by id, stamped with the current round
the round ends → the structure read (session + round) → the structure cache, then claim by hash
```

### Invariants

1. A component never holds a connection: every api call is a `Session` method.
2. A change lands locally before it is sent.
3. Blocks are addressed by id, never by position.
4. The window's size follows `config.ui.session.preloadCache`, not the conversation.
5. The tail is not a cache of finished rows.
6. Only what the backend pushed and what a surface asked for by name comes in.
7. The content hash has one recipe (`ContentKey` in storage).

## 2. Design intent

**Entry and queue**

- **One entry point** — so no front end invents a second way of asking.
- **A change and a question are different** — a change carries no answer, so it lands at once and
  the reader never waits a round trip; a question waits.
- **One queue** — intents from any component go out in one batch per frame, in order; the order is
  the semantics.

**The read model**

- **The view is borrowed** — drawing every frame cannot copy the transcript.
- **The window is bounded** — memory follows the window, not the conversation.

**Two caches**

- **They are separate** — a block can be managed by two rounds, so the structure cannot be mixed into
  the content.
- **The structure holds ids and hashes only** — a round carries tens to hundreds of blocks, so 8
  rounds fit only because nothing but numbers is kept.
- **Ids are claimed by content hash** — block ids are sequential, so matching by position is not
  reliable; matching by content is.
- **The finish signal carries a flag** — without it a front end cannot tell an increment from the
  finished block.
- **The round id comes back with the submission** — a front end knows which round it is writing into
  without waiting for the round to end.

**Tail and placeholder**

- **The tail is not a finished-row cache** — it changes at streaming rate; a render side memoizes it
  against `LiveTail::version`.
- **One optimistic row** — an interrupt stands a row where the server's own will land, so the screen
  does not wait out a dying round.

**State**

- **The task list has one starting point** — the api answers with the list as it stands; everything
  after that arrives as the blocks the front end already receives.
- **The working directory is compared, not remembered** — the row's `cwd` is where the last `/cdp`
  landed and the live one is where the model's `cd` has taken the session; equal means it was not
  moved, different means the model moved it for now.
- **Older and newer reads may name a round** — blocks of that round come back without a round of
  their own.

## 3. Unresolved

1. The tree, the round list, replay and logs have no api. The structure cache keeps 8 rounds; a tree
   over a whole session needs a read beyond that.
2. A context estimate has no api: before a round has run there is no measured prompt size.
3. The layering rules do not know this tree: the architecture test forbids a front end from naming
   the storage and agent layers, while the api hands out `Block`, `NewBlock`, `TurnBlock` and
   `TodoPhase` directly.
4. `ui` is not in the build: `lib.rs` has no `pub mod ui;`, and `Cargo.toml` has no `ratatui` /
   `crossterm`.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.session.preloadCache` | how many blocks the content cache keeps around the view (default 128) |
| `config.ui.placeholder.interrupted` | the words the optimistic interrupt row carries |
