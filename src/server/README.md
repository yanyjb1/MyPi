# server

The server side of mypi: the process that owns the database, the agent loop and the configuration, and answers a front end over a wire; nothing about a front end or a command line lives here.

## 1. The design

### Four domains, and the wire

1. `api` — the surface a front end talks to: ask, change, subscribe; its doc is `api/README.md`.
2. `storage` — the database and its one writer, which answers no query; its doc is `storage/README.md`.
3. `agent` — one round: the loop, the request half and the tools; its doc is `agent/README.md`.
4. `config` — the two files, the profiles and the built-in texts; its doc is `config/README.md`.

The wire lives beside this folder (`../ipc/`), because both halves use it, and the operating system sits below all of it in `../platform/`; `ipc/README.md` and `platform/README.md` are their docs.

### How the pieces sit

Directions are one way, downward only:

```text
  the entry point (src/main.rs): `mypi --server`, or the server a bare `mypi` becomes
        │
        ▼
      ipc ──► api ──► agent ──► storage
                │        │         │
                │        └─────────┴──► platform
                └──► config ──────────► platform
  (every arrow points down; nothing points back up)
```

Nothing points back up, and the guard table in `src/lib.rs` (`mod architecture`) is that rule as a failing test.

### What the server holds in memory

The api's state (the writer, every open session's two configs, the running rounds, the relay and the config), the config itself, the time zone, the log ring, and the wire's own bits (the database lock, the endpoint and the register of front ends); everything else is the database or a file.

### Invariants

1. **One writer.** One thread ever writes the database, and one session runs one round at a time.
2. **The api is the only surface.** Nothing else under `server/` is a front end's business.
3. **One mouth per question.** Storage writes every row, config reads every file, platform asks the operating system, and the loop runs rounds.
4. **The loop reads a session once, then appends**, so a restarted process sends the same bytes.
5. **The server renders nothing**, and a front end holds no server state.

## 2. Design intent

1. **One mouth per question.** The database, the files, the operating system, the loop and the front end's surface are five problems with five rates of change, and giving each one owner makes "where does this belong" independent of who is asking.
2. **The direction rule is a test, not a convention.** The guard table in `src/lib.rs` fails a test when a layer reaches the wrong way.
3. **The api is the only surface.** A front end learns one vocabulary and the wire carries the api's values, because a second edge would only translate them.
4. **The loop reads a session once.** It seeds a session's history when the session opens, appends each block after it lands, and reads it again only after a rewind or a compaction, because identical bytes across laps, restarts and resumes are what a prefix cache is paid for.
5. **The server holds no conversation of its own.** The loop's transcript is a cache of the rows and the database stays the authority, so a rewind or a compaction reads it again; state has two homes, the database and the config, and what is in memory is the residue neither can carry.
6. **The server stops when nobody is watching.** A front end's death is noticed by its connection rather than by a message, so nothing has to be timed out.
7. **Nothing in the server renders.** Keeping presentation out means the same content serves any front end, and the server never has to know a screen's width.

## 3. Open questions

1. **Windows.** The named pipe is unwritten and several places are unix-shaped (第 16 条, 第 40 条).
2. **Nothing reconnects** after the server dies (第 42 条), and **a front end cannot read the server's log** (第 41 条).
3. **The config is read once, at startup**, with no hot reload yet (第 20 条).
4. **One-shot commands bypass the wire**, because `mypi --sessions` opens its own read connection.

Each domain's own list (its README's §3) carries the rest.

## 4. Config

The server side reads `config.server.*` and `config.ui.tui.*` belongs to the front end; this layer opens no file, and the sources and the naming rules are in `config/README.md`.

1. `config.server.api.*` — the default model.
2. `config.server.request.*` — how much of a running round a front end sees, and the compaction's tail and template.
3. `config.server.agent.*` — the tool-round fence, the tool budgets and limits, and the profile.
4. `config.server.storage.*` — how many completed tool rounds land one batch.
5. `config.server.platform.*` — the time zone, the stamp templates and the endpoint kind.
6. `models.*` — providers, endpoints, keys and prices.
7. `profile.*` — the enabled profile's system prompt, tool list and compaction instruction.

## See also

`ARCHITECTURE.md` — the files, the relationships and the threads.