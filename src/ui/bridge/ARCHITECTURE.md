# bridge — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

1. `mod.rs` is the module list and the three exports.
2. `run.rs` is `Options` and `run`: the listener, the accept loop, the sweep, and the adapters' own programs.
3. `manager/` holds the conversations the door keeps: `hub.rs` the table, `held.rs` one connection's question, its listeners and its round.
4. `http/` is the door itself: `read.rs` one request, `respond.rs` one answer, `serve.rs` one connection answered, `route.rs` who may come, `rpc.rs` one body to a frame, `local.rs` the door's own look at this machine, `title.rs` the name a cited page gives itself, `host.rs` the door's own keeping of a session.
5. `adapter/` is everything that speaks for a front end: `manifest.rs` reads `adapters.yml`, `events.rs` is what one adapter is fed, `web.rs` and `web/` are the webui.

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
- `Held::book` is the promise that a connection stays open with nobody watching it, and the whole of what hosting is; `Held::unbook` takes it back. `Hub::host` books the connection it opens, `Hub::watch` unbooks the one it hands out, so any front end following the session takes it back. `Hub::kept` is the table read for the connections still booked, with whether a round runs in each.
- `Hub::sweep` lets go of a conversation whose connection died, and of one with no listener, no running round and no booking after ten minutes; letting go means a `watch` with no session and a `bye`, which unbooks it on the server along with its jobs and browser tabs.
- `Held::close` shuts the socket down after its last words, which is what wakes the blocked reader.
- `follow` keeps a second handle on the peer's socket and reads it in its own thread: the read returns only when the peer leaves, and that is when the listener it added is taken off the conversation again. Without it a closed browser tab would keep its conversation held forever.
- `serve` reads the method first: `GET` is a file of `webui/`, answered without naming any adapter. `POST` names its adapter in the head, is checked against that adapter's rules, becomes a frame, and then either opens a stream — the frame is `subscribe` / `watch` — or is answered and closed. Frames of kind `local` and `host` are answered by the door itself and never cross.
- `events::take` is the one place an adapter's declarations are honoured: pieces it switched off go nowhere, and a text piece is cut to its own length. Nothing is assembled: a round's frames reach the adapter as they arrived.

## The webui

1. `index.html` is the shell: the three columns, the dial's own drawing, and every fixed shape as a `<template>` a builder clones.
2. `styles/main.css` is the one entry. It declares the `@layer` order and imports `01-reset`, `02-tokens` (both palettes and every length), `03-layout` (the three columns and the two drawers), `04-primitives` (the shapes every region reuses), then the five in `styles/components/` — `rail.css`, `chat.css`, `composer.css`, `dial.css`, `panel.css`, one sheet a region.
3. `js/core/` is what every region stands on: `api.js` (the one socket, every read and write over it, the conversation followed, and the model table), `store.js` (the state, the one announcement, and the conversation this tab keeps), `template.js` (a template cloned and filled by name, and a panel's own markup and sheet taken in), `theme.js`, `format.js`.
4. `js/render/` turns a body into HTML: `markdown.js` cuts a block into runs and paints each on its own, `tools.js` draws the tool card and holds the registry a tool claims.
5. `js/components/` holds one region a file: `rail.js` the session list and the two dialogs it asks with, `panels.js` the drawer's host and registry, `dial.js` the ring and the words on it. `chat/` is the transcript — `list.js` enters a conversation, opens a draft and draws the head above, `sheet.js` lays one page and its tail, `window.js` decides which rounds are built, what is read ahead and what is given up, `scroll.js` keeps the reader's place, `stream.js` folds a frame in, `live.js` says the round being said now, `note.js` is one line a notice takes. `composer/` is the box — `input.js` the directory field and the first draw, `keys.js`, `say.js`, `offer.js`, `path.js`, `summon.js`, `talk.js`, and `mods.js` the capsule under it.
6. `panels/` holds one panel a directory, its markup, its sheet and its script together: `usage/`, `memo/`, and `citation/`, whose pills are the prose's own and whose way in is a press on one of them.
7. `js/main.js` is the boot: the registration, the controls, the first frames.

The page holds no framework and no third-party script: the browser loads the modules as they are. Every file is seeded into `webui/` under the global config directory on the first start and served from disk afterwards, so a reader changes the look by editing a sheet and nothing else. The page's own directory is the only place a file is named: `web.rs` embeds it whole.

`store.js` holds the page's state and the one announcement that follows a change; every other module writes into what it holds and then announces. A frame from `stream` reaches `stream.js` first, which decides what it changes. A fixed shape is a `<template>` and a builder clones it and fills it by the name each part carries; a panel brings its own markup and its own sheet, both taken in the first time it opens.

A round's frames arrive live and are thrown away when the round closes: the receipt carries the ranges, the page asks `call blocks` for those, and the transcript is then drawn from what storage holds. That is why a rewind or a reload shows the same history the terminal does.

The page holds one socket. `api.js` dials it, puts an `id` on every question, settles the reply carrying that id, and hands every other frame to the conversation it follows; a socket that breaks is dialled again on a rising delay and the watch is sent again, which is all a reconnect needs because the door replays the round in flight. When nothing follows and nothing waits, the socket is closed, since a connection is what keeps a conversation alive on the server.

The right drawer is a column where there is room and a sheet where there is not: narrow, it comes up from the floor, its grip drags it back down, and it keeps its place in the layout rather than being taken out of it, so its way in and out is one movement.

The dial reads the round list the transcript does and shows five rounds thirty degrees apart, the round in view always on the middle bearing: the wheel moves that round, never the ring. A wheel is counted by the distance it says rather than by the event, one notch per enough of it, and a finger turns the ring once it has moved far enough to be dragging rather than tapping. Each notch carries its round's own opening, measured with the page's font, and a round the transcript has dropped is asked for by `call blocks`.

The three controls above the box speak for a session or for the draft. `#cwd`, `#model` and `#effort` show the session's own directory, model and rung; a draft keeps the same three slots, and its first message carries them into `post init`. The effort capsule offers only the rungs `models.model.<address>` declares, `auto` first.

## Must stay in sync

1. `adapters.yml`'s field names and `manifest::Entry`: `serde` refuses an unknown field, so a renamed field stops the bridge instead of being ignored.
2. `Rule::parse`'s three kinds and the api's `route.rs`: a new kind on the wire is a new word here.
3. `events::take`'s piece kinds and the relay's live payloads: a new `kind` needs its place on the `THINKING`/`TOOLS` lists, or it crosses every adapter unfiltered.
4. `closes_frame` and the receipt the relay sends: the frame that ends a round is the one carrying `ranges`.
5. `chat/stream.js`'s frame kinds and the relay's live payloads: a piece the page does not name is a piece it drops.
6. One address, two methods: `GET` for a file and `POST` for a frame, and `README.md`'s own list.
7. `rpc::KINDS` and the door's own branches in `http/serve.rs`: `local` and `host` are the two kinds that never reach the server, and `Rule::parse` knows only the three that do.
8. `web.rs`'s embedded directory and `web/`: a file written there is served without an edit; a name the door must serve is one the page's own `kind` knows.
9. `main.css`'s `@layer` order and every sheet: a sheet names the layer it joins, and the order is declared once.
10. `render/markdown.js`'s `cite-pill` and `panels/citation/`: the class is the one word the prose and the drawer share, and a pill that stops carrying it stops opening anything.

## Not wired

1. `rpc::is_watch` is the one frame the door turns into a stream: `subscribe` / `watch` that names a session. Any other `subscribe` name is answered like a question.
2. Nothing checks a `forbid` name against the server's own table: a mis-spelled name closes nothing.
3. The page's files take no adapter name, so `adapters.yml` decides who may use the door but not who may load the page.
4. The page keeps its blocks in the browser and only reloads the ones a round closes; a session left open across many rounds grows with them.