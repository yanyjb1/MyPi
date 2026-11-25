# bridge — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds |
|---|---|
| `mod.rs` | the module list and the three exports |
| `run.rs` | `Options`, `run`: the listener, the accept loop, the sweep, the adapters' own programs |
| `manager/` | the conversations the door holds: `hub.rs` the table, `held.rs` one connection's question, listeners and round |
| `http/` | the door itself: `read.rs` one request, `respond.rs` one answer, `serve.rs` one connection answered, `route.rs` who may come, `rpc.rs` one body to a frame, `host.rs` the door's own look at this machine |
| `adapter/` | everything that speaks for a front end: `manifest.rs` `adapters.yml`, `events.rs` what one adapter is fed, `web.rs` and `web/` the webui |

## What it talks to

- **`ipc::client`**: `Client::connect`, `send`, `next`, `drain`, `alive`, `close` — one connection per held conversation, and one more that follows nothing.
- **`ipc::contract`**: `Frame`, and nothing of its payloads.
- **`ipc::endpoint`**: where the server listens. The bridge does not start the server.
- **Knows nothing about**: the backend, the database, the agent loop, the terminal.

## Inside

```text
main
 ├── Config::paths().global_root()  ── where adapters.yml and webui/ live
 └── Endpoint::file_path / spawn_server  ── where the server listens

bridge::run
 ├── Hub::bare         ── one connection that follows nothing: the idle server stays alive
 ├── TcpListener       ── localhost, or the whole network under --lan
 ├── start_adapters    ── one program per adapter that named a command
 ├── accept            ── one thread per request
 └── wait_for_stop     ── a signal ends the programs, then every conversation
```

- `Hub::get` opens a conversation's connection on first use and watches that conversation; `Hub::bare` is the connection every question without a `session` uses. Each carries its own viewer name, so the server sees one front end per conversation.
- `Held::ask` takes the connection's one question slot, sends the frame, and waits for the reply that carries that kind and name. `Held::read` blocks on `Client::next` and hands a matching reply to the question and every other frame to the listeners.
- `Held::fanout` keeps a round's frames until the receipt that carries `ranges` arrives, so a listener joining mid-turn sees the round from its first piece.
- `Hub::sweep` lets go of a conversation with no listener and no running round for ten minutes; letting go means a `watch` with no session and a `bye`, which unbooks it on the server along with its jobs and browser tabs.
- `Held::close` shuts the socket down after its last words, which is what wakes the blocked reader.
- `follow` keeps a second handle on the peer's socket and reads it in its own thread: the read returns only when the peer leaves, and that is when the listener it added is taken off the conversation again. Without it a closed browser tab would keep its conversation held forever.
- `serve` reads the method first: `GET` is a file of `webui/`, answered without naming any adapter. `POST` names its adapter in the head, is checked against that adapter's rules, becomes a frame, and then either opens a stream — the frame is `subscribe` / `watch` — or is answered and closed. A frame of kind `host` is answered by the door itself and never crosses.
- `events::take` is the one place an adapter's declarations are honoured: pieces it switched off go nowhere, and a text piece is cut to its own length. Nothing is assembled: a round's frames reach the adapter as they arrived.

## The webui

| path | holds |
|---|---|
| `adapter/web/index.html` | the page's template: the rail, the transcript, the composer's bar and box, and the panel's empty frame |
| `adapter/web/css/base/` | `base.css` (the palette of `R G B` tokens and the reset), `bits.css` (the reusable shapes), `layout.css` (the three columns and the two drawers) |
| `adapter/web/css/rail/` | the left column's own sheet |
| `adapter/web/css/chat/` | the middle column's sheets: `sheet.css` for the transcript, `deck.css` for the composer, `dial.css` for the ring |
| `adapter/web/css/panel/` | the right column: the panel's tabs, `bill.css` the drawer of a lap's bill, and the body a plugin fills |
| `adapter/web/js/base/` | what every column shares: `dom.js` (the one node builder), `model.js` (a model's declared rungs), `note.js` (one-line text and numbers), `state.js` (what the page knows), `wire.js` (the door), `read.js` and `write.js` (every `call` and every `post`), `theme.js` (the two palettes) |
| `adapter/web/js/rail/` | the left column: `rail.js` the session list, `ask.js` the two dialogs it asks with, `draft.js` a draft becoming a session |
| `adapter/web/js/chat/history/` | the transcript: `sheet.js` its drawing, `rounds.js` and `card.js` one round and its cards, `mark.js` a body to HTML, `window.js` which rounds are built, `cap.js` the quota and `trim.js` what it gives up, `live.js` how a frame is folded, `enter.js` entering a conversation |
| `adapter/web/js/chat/composer/` | the box: `bar.js` the bar of directory, model, effort and bill, `ring.js` the ring it carries, `say.js` its text and chips, `offer.js` and `path.js` what a word offers and how one is picked, `key.js` the keys claimed, `summon.js` the `/` words, `talk.js` sending |
| `adapter/web/js/chat/dial/` | the ring: `dial.js` five bearings thirty degrees apart, `words.js` a round's opening set along its notch's own line |
| `adapter/web/js/panel/` | the right column: `panels.js` the registry, `bill.js` the drawer a lap's bill fills, `plugins/` the six panels (`usage`, `todo`, `memo`, `tree`, `child`, `setup`) |
| `adapter/web/js/main.js` | the boot: the registrations, the controls, the first frames |

The page holds no framework and no third-party script: the browser loads the modules as they are. Every file is seeded into `webui/` under the global config directory on the first start and served from disk afterwards, so a reader changes the look by editing a sheet and nothing else.

`state.js` holds the page's state and the one announcement that follows a change; every other module writes into what it holds and then announces. A frame from `stream` reaches `live.js` first, which decides what it changes.

A round's frames arrive live and are thrown away when the round closes: the receipt carries the ranges, the page asks `call blocks` for those, and the transcript is then drawn from what storage holds. That is why a rewind or a reload shows the same history the terminal does.

The dial reads the round list the transcript does and shows five rounds thirty degrees apart, the round in view always on the middle bearing: the wheel moves that round, never the ring. Each notch carries its round's own opening, measured with the page's font, and a round the transcript has dropped is asked for by `call blocks`.

The three controls above the box speak for a session or for the draft. `#cwd`, `#model` and `#effort` show the session's own directory, model and rung; a draft keeps the same three slots, and its first message carries them into `post init`. The effort capsule offers only the rungs `models.model.<address>` declares, `auto` first.

## Must stay in sync

1. `adapters.yml`'s field names and `manifest::Entry`: `serde` refuses an unknown field, so a renamed field stops the bridge instead of being ignored.
2. `Rule::parse`'s three kinds and the api's `route.rs`: a new kind on the wire is a new word here.
3. `events::take`'s piece kinds and the relay's live payloads: a new `kind` needs its place on the `THINKING`/`TOOLS` lists, or it crosses every adapter unfiltered.
4. `closes_frame` and the receipt the relay sends: the frame that ends a round is the one carrying `ranges`.
5. `live.js`'s frame kinds and the relay's live payloads: a piece the page does not name is a piece it drops.
6. One address, two methods: `GET` for a file and `POST` for a frame, and `README.md`'s table.
7. `rpc::KINDS` and the door's own branch in `http/serve.rs`: `host` is the one kind that never reaches the server.
8. `web.rs`'s seed table and `web/`: a file added to the page is a file the table must name.

## Not wired

1. `rpc::is_watch` is the one frame the door turns into a stream: `subscribe` / `watch` that names a session. Any other `subscribe` name is answered like a question.
2. Nothing checks a `forbid` name against the server's own table: a mis-spelled name closes nothing.
3. The page's files take no adapter name, so `adapters.yml` decides who may use the door but not who may load the page.
4. The page keeps its blocks in the browser and only reloads the ones a round closes; a session left open across many rounds grows with them.