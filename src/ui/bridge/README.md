# bridge

An HTTP door onto the server, for every front end that is not this terminal: a browser page, a chat bot, a script. It holds several conversations at once, which the IPC wire does not.

**`API.md`, beside this file, is the one a front end is written against: every frame and every name the door carries.**

## 1. Design / Schema

### The door

One TCP port and nothing behind it: no path, no query, no second address. What a request is asking for is said by its method.

| method | asks for |
|---|---|
| `GET /<file>` | one file of the webui directory; `/` is the page itself |
| `POST /` | one frame, and the server's answer as the reply |

A `POST` names its adapter in the `X-Bridge-Adapter` head; there is no other way to say it. A name the file does not carry is refused with `403`. The body is `{kind, name, payload}`, carried as it arrived: `call`, `post` and `subscribe` cross unchanged, and the answer is the payload the server wrote.

One kind is the door's own: `host` / `list` answers the names one directory holds, plus the directory it read, and never crosses to the server. The page uses it for path completion, which a browser cannot take on its own.

One frame is answered with a stream instead of a reply: `subscribe` / `watch`, which starts following a conversation. That connection then stays open and carries that conversation's frames, one per line, until the peer leaves. Which conversation it is comes from the frame's own `session`; the door adds nothing and asks for nothing else.

An error is `{"error": {"code", "message"}}` when the door itself refused. A refusal the server made arrives as that server's own error payload, untouched.

### What an adapter is fed

Every adapter declares which categories of live output it wants, and how long a text piece may be. The door then keeps what was asked for and drops the rest.

| declaration | effect |
|---|---|
| `thinking` | `thinking` and `boundary` pieces are forwarded, or dropped |
| `tools` | `arguments` and `tool_*` pieces are forwarded, or dropped |
| `chunk` | a `text` piece longer than this many characters arrives as several frames; `0` is no limit |

Only the categories are separated. Whether a tool call is paired with its result, how long a round was, and how the words are laid out are the receiver's own business: the door forwards frames, it does not assemble new ones.

### The file

`adapters.yml`, in the global config directory, and read nowhere else. The bridge reads it once, at startup, and does not start when it is missing, unreadable, or badly shaped: a door with no list of adapters serves nobody.

```yaml
port: 8737
adapters:
  webui:
    forbid: [post/delete]
  telegram:
    thinking: false
    tools: false
    chunk: 4000
    local-only: true
    command: "python3 ~/bots/telegram.py"
    forbid: [post]
```

| field | default | meaning |
|---|---|---|
| `port` | `8737` | the one port every adapter shares |
| `local-only` | `false` | refuse a client from anywhere but this machine |
| `thinking` | `true` | whether `thinking` and `boundary` pieces are forwarded |
| `tools` | `true` | whether `arguments` and `tool_*` pieces are forwarded |
| `chunk` | `0` | the most characters one `text` frame carries; `0` is no limit |
| `command` | none | a program the bridge starts beside itself and stops with it |
| `forbid` | none | what this adapter may not send, written as `kind` or `kind/name` |

`forbid` holds only what is refused, never what is allowed: an adapter may say everything the server names, except what is written there. `post` closes the whole kind; `post/delete` closes one name inside it. The three kinds are `call`, `post` and `subscribe`; a rule names a kind and at most one name, so `post/meta` is the deepest a rule goes.

### The webui

The one front end that is written here rather than declared. Its files are seeded into `webui/` under the global config directory on the first start and served from disk afterwards, so a reader may edit the page without rebuilding anything; a deleted file falls back to the built-in copy and is written back on the next start.

The page has no framework, no bundler and no third-party script: one HTML template, `css/` with one sheet per region, and `js/` with the plain modules the browser loads.

A running round is laid down a segment at a time. A block's body is cut into runs — prose, a fence, a table, a quote, a list — and only the open tail is ever cut again, so a settled run parses once and keeps the nodes it was given; a stream frame repaints that tail and nothing else on the page.

The composer's bar carries the three facts a turn depends on, left to right: the directory, the model and the thinking effort. The model is a native select over `call pools`'s list. The effort is one capsule with a stop per rung `models.model.<address>` declares, `auto` first; a model that declares none shows `auto` alone. A draft keeps the same three slots, and its first message carries them into `post init`.

A lap's bill rides the last piece it wrote, and a zeroed one is the provider stating a fact rather than a bill: the drawer's figures and the composer's ring are read again at every real quote, so both follow a running round instead of holding whatever the last entry left.

### The door's own switch

`mypi --bridge` listens on this machine alone and serves the webui at that address. `mypi --bridge --lan` opens the same port to the local network. An adapter that wrote `local-only: true` still refuses a client from the network, so one port can serve a local bot and a browser across the room at once.

## 2. Design intent

- **The postman does not open the mail.** Names and payloads cross unchanged; what a frame means is the front end's business. The bridge reads a `kind` and a `name` for one decision — the adapter's own rules — and nothing else.
- **Categories, not shapes.** An adapter declares which kinds of piece it can receive. Assembling, pairing and laying out are what the receiver does with them.
- **One connection per conversation.** The server keeps a conversation's runtime only while some connection follows it, and one connection follows one conversation, so holding several is holding several connections.
- **A conversation is held while it has a reason to be.** A listener on its stream, or a round it has not finished, is that reason; a conversation with neither for ten minutes is handed back.
- **One stranger at a time per conversation.** A reply names only its kind and its name, so two questions in flight on one connection could take each other's answers. Each conversation therefore answers one question before the next is sent.
- **One program, one door.** An adapter's own program is started and stopped with the bridge, so a bot does not outlive the process that fed it.
- **One address.** A front end reaches the door by connecting to it; nothing about what it wants lives in the path.

## 3. Unresolved

1. A name mis-spelled in `forbid` closes nothing and says nothing: the bridge keeps no copy of the server's name table, by design, so it cannot check one.
2. `--lan` has no authentication and no encryption: anybody on the network who names a declared adapter may use whatever that adapter may use.
3. The webui keeps a conversation's blocks in the browser; a rewind or a compaction asks for them again, and a page left open across a long session grows until it is reloaded.

## 4. Config fields

| key | use |
|---|---|
| `adapters.yml`'s `port` | the port the door listens on |
| `config.server.platform.ipc` | not read here: `main` picks the endpoint the door connects to |