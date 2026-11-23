# bridge — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds |
|---|---|
| `mod.rs` | the module list and the three exports |
| `manifest.rs` | `Manifest`, `Adapter`, `Rule`: `adapters.yml` read once, and whether one request is refused |
| `hub.rs` | `Hub` and `Held`: the conversation table, one upstream connection each, its question in flight, its listeners, its round |
| `events.rs` | `take`: one frame kept, dropped, or cut to length, by what the adapter declared |
| `run.rs` | `Options`, `run`, `serve`: the listener, one thread per request, the files, the frames, the adapters' own programs |
| `http.rs` | `Request`, and the words of HTTP: one request read, one answer written, one stream held open |
| `route.rs` | which adapter a request speaks for, and whether it is admitted |
| `rpc.rs` | one request body to a frame, one frame that starts a stream, and a name the adapter may not send |
| `web.rs` | `Webui`: the page's files, seeded onto disk and answered from there |
| `web/` | the webui itself: one HTML template, `css/` per column, `js/` per module |

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
- `serve` reads the method first: `GET` is a file of `webui/`, answered without naming any adapter. `POST` names its adapter in the head, is checked against that adapter's rules, becomes a frame, and then either opens a stream — the frame is `subscribe` / `watch` — or is answered and closed.
- `events::take` is the one place an adapter's declarations are honoured: pieces it switched off go nowhere, and a text piece is cut to its own length. Nothing is assembled: a round's frames reach the adapter as they arrived.

## The webui

| path | holds |
|---|---|
| `web/index.html` | the page's template: the rail, the transcript, the composer and the panel's empty frame |
| `web/css/base.css` | the palette of `R G B` tokens, the reset, and the three words every zone shares |
| `web/css/rail.css` | the left column: the session list and the directory a new one starts in |
| `web/css/main.css` | the middle column: the head, the transcript, the live rows, and the composer |
| `web/css/panel.css` | the right column: the panel that opens over the transcript and closes again |
| `web/js/wire.js` | the door: one question, one stream, and the refusal both sides speak |
| `web/js/text.js` | text as rows: a body to HTML, and the one line a collapsed tool card shows |
| `web/js/state.js` | the page's state: the rail, one conversation's rounds, and the frames folded into them |
| `web/js/view.js` | the one render, from the state, and the panels |
| `web/js/app.js` | the boot: the controls, the first frames, and the redraw |

The page holds no framework and no third-party script: the browser loads the modules as they are. Every file is seeded into `webui/` under the global config directory on the first start and served from disk afterwards, so a reader changes the look by editing a sheet and nothing else.

`state.js` is the only writer of the page's state, and `view.js` is the only reader. A frame from `stream` reaches `state.js` first, which decides what it changes; nothing in the view reads a payload.

A round's frames arrive live and are thrown away when the round closes: the receipt carries the ranges, the page asks `call blocks` for those, and the transcript is then drawn from what storage holds. That is why a rewind or a reload shows the same history the terminal does.

## Must stay in sync

1. `adapters.yml`'s field names and `manifest::Entry`: `serde` refuses an unknown field, so a renamed field stops the bridge instead of being ignored.
2. `Rule::parse`'s three kinds and the api's `route.rs`: a new kind on the wire is a new word here.
3. `events::take`'s piece kinds and the relay's live payloads: a new `kind` needs its place on the `THINKING`/`TOOLS` lists, or it crosses every adapter unfiltered.
4. `closes_frame` and the receipt the relay sends: the frame that ends a round is the one carrying `ranges`.
5. `web/state.js`'s frame kinds and the relay's live payloads: a piece the page does not name is a piece it drops.
6. One address, two methods: `GET` for a file and `POST` for a frame, and `README.md`'s table.

## Not wired

1. `rpc::is_watch` is the one frame the door turns into a stream: `subscribe` / `watch` that names a session. Any other `subscribe` name is answered like a question.
2. Nothing checks a `forbid` name against the server's own table: a mis-spelled name closes nothing.
3. The page's files take no adapter name, so `adapters.yml` decides who may use the door but not who may load the page.
4. The page keeps its blocks in the browser and only reloads the ones a round closes; a session left open across many rounds grows with them.