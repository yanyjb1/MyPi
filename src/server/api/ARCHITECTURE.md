# api architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations, and what this layer exports: the doors' shapes, `StreamItem`, `Opened`, `Viewer` |
| `types.rs` | `Call`, `Post`, `PostDone`, `Subscribe`, `Reply`, `Viewer`, `ModelId`, `UsageLang`, `CandidatePools` |
| `driver/mod.rs` | the api's state: the writer, the session table, the running set, the relay, the config; `state(cfg)` builds it once; `call`, `rewind`, `interrupt`, watching, and the two doors that open a stretch |
| `driver/sessions.rs` | the session table: one `Live` per session, built on demand; `drop_live` lets one go |
| `driver/running.rs` | which sessions are mid-round, and the flag that stops them |
| `driver/relay.rs` | storage's one port, and the fan-out to subscribers — per session, keyed by viewer |
| `driver/round.rs` | one round on its own thread, the outlet that streams it, and letting the state go at its end |
| `driver/compact.rs` | the compaction's machinery: gather, plan, summarize, land, and settle one that never lands |
| `call/mod.rs` | the call door: reach the state, open a read connection, dispatch, close |
| `call/sessions.rs` | the picker's rows |
| `call/viewport.rs` | the window: first frame, older, newer |
| `call/usage.rs` | the bill, priced here |
| `call/complete.rs` | the two completion pools |
| `call/todo.rs` | the todo list, replayed from the session's own tool calls |
| `call/meta.rs` | the session row, plus the working directory in memory |
| `post/mod.rs` | the post door: reach the state, dispatch, and finish the changes the state itself owns |
| `post/init.rs` | creating a session before any message |
| `post/name.rs` | `/name` |
| `post/cwd.rs` | `/cdp` |
| `post/model.rs` | `/switch` and `/model` |
| `post/profile.rs` | `/profile`, resolved against the roster |
| `post/delete.rs` | `/delete` and `/rm` |
| `post/rewind.rs` | the cut (prune mode; the fork mode is a `todo!`) |
| `subscribe/mod.rs` | the subscribe door: `watch`, the `Opened` verdict, and its two stretch entries |
| `subscribe/submit.rs` | `SubmitOp` — open a round |
| `subscribe/compaction.rs` | `CompactOp` — summarize a context region |
| `reply/mod.rs` | what the doors send back |
| `reply/stream.rs` | `StreamItem`, and `Relayed` (a piece off storage's port) |

## The public surface

| item | what it does |
|---|---|
| `call::answer(cfg, call)` | answers a `Call`, on a read connection the state opens |
| `post::apply(cfg, post)` | carries a `Post` out |
| `subscribe::watch(cfg, viewer, session)` | one viewer's stream for one session |
| `subscribe::submit(cfg, session, prefix, text, suffix)` | opens a round; answers started / busy |
| `subscribe::compact(cfg, session, retention, focus)` | opens a compaction; answers started / busy |
| `Opened` | what the two stretch doors answer with: `Started` or `Busy` |
| `Retention` | how much of the tail a compaction keeps, as the surface parsed it |
| `Call`, `Post`, `Subscribe`, `Reply`, `PostDone` | the door shapes |
| `StreamItem` | the stream vocabulary; `Ended` names the round it closed |
| `Viewer` | a front end's own id, carried on every claim |
| `ModelId`, `CandidatePools` | the shared shapes a surface also names |

`Post::Unwatch` is the other half of a claim: it drops one viewer's stream and asks whether the
session's state should go with it.

The two doors' contracts (`SubmitOp`, `CompactOp`) are this layer's own: they state what the api's
state must supply, and nothing outside implements them.

## Relationships

```text
  front end ──subscribe::watch────────────┐  (a stream, per viewer)
             ──call::answer───────────────┤
             ──post::apply (…Unwatch)─────┤
             ──subscribe::submit/compact──┤
                                          ▼
                                   ┌─ state ─┐
                storage Writer ◄────┤ sessions│  (one Live per session)
                storage Reader ◄────┤ running │  (who is mid-round)
                                    │ relay   │  (who is watching)
                                    └────┬────┘
                                         │ round thread
                                         ▼
                  agent::loop::run(writer, connection, live, turn, outlet)
                                         │
                      storage Writer ◄───┘ (blocks, archives, round end)
                                         │
                         storage Relay ──┴─► the fan-out ──► each viewer's stream
```

- **Who calls the api**: a front end, and nothing else. It names the three doors and holds no state
  of the api's.
- **Who the api calls**: `storage` (the writer, a read connection, the decoders), `config` (values
  only), `platform` (the clock, the log, the data directory), and `agent` (the loop, the tools, the
  request side, the compaction plan).
- **Who reads the database**: `call/*`, `post/*`, and the state's session table — each opens its own
  connection, writes its own SQL, and decodes with `storage::decode`.
- **Who the api does not know**: a front end, the command line, the wire protocol, and the shape of
  a request body.
- **The contract** (`crate::commands`) is read by the api for the marker types the session commands
  implement; the api never parses a command line.

## Threads

| thread | created by | lives |
|---|---|---|
| the writer | `state()` (through `storage::Writer::open`) | until the process ends |
| the relay | `state()` | until the process ends; hands storage's pieces to the viewers' streams |
| a round | `state.submit` (through `subscribe::submit`) | one round: borrows the session's `Live`, runs the loop, gives it back or lets it go, releases the claim |
| a compaction | `state.compact` | one summarization round, same borrowing rules |

Two locks guard the state's shared parts: the session table and the running set — plus the relay's
viewer list. They are taken one at a time, never nested. Dropping a `Live` (and with it the temp
files of a session's artifacts) happens on the round's own thread, after the claim is released.

## Parts that must stay in step

1. `call/sessions.rs`'s `SESSION_COLS` and `storage::decode::session_meta` — the columns are read by
   position, and `call/meta.rs` shares the same constant.
2. `call/todo.rs`'s replay and the `todo` tool's own operations: the replay runs the tool, so a rule
   about what an operation does lives in one place only.
3. `driver/sessions.rs`'s `Settings` assembly and `agent::loop::Settings` — every field the loop
   reads is filled here.
4. `driver/round.rs`'s `Preview` and `reply/stream.rs`'s `StreamItem` — what the loop gives out is
   what a viewer receives. The streamed text reaches storage as `Submit::Blocks { done: false }`,
   and the request side's cut is what closes it.
5. `types.rs`'s `Subscribe` variants and the door entries — one variant per entry (watch, submit,
   compact).
6. The two questions that reap a session — `relay.watched` and `running.flag` — and every place that
   asks them (`driver/mod.rs::reap`, `driver/round.rs`, `driver/compact.rs`).

## Not wired yet

1. The entry points reach the ipc layer directly; nothing calls these three doors (第 8 条).
2. Nothing refreshes the completion pools or the tool roster after a config reload (第 20 条).
3. The fork mode of a rewind is a `todo!()`.
