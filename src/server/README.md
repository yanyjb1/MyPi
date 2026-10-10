# server

The server side of mypi: the process that owns the database, the agent loop, the configuration and the
operating system, and answers a front end over a wire. Nothing about a front end or a command line
lives here.

## 1. The design

### Five domains, and the wire

| domain | what it is | its doc |
|---|---|---|
| `api` | the surface a front end talks to: ask, change, subscribe | `api/README.md` |
| `storage` | the database and its one writer; it answers no query | `storage/README.md` |
| `agent` | one round: the loop, the request half, the tools | `agent/README.md` |
| `config` | the two files, the profiles, the built-in texts | `config/README.md` |
| `platform` | the only place that asks the operating system | `platform/README.md` |

The wire lives beside this folder (`../ipc/`), because both halves use it: `ipc/README.md`.

### How the pieces sit

Directions are one way — downward only:

```text
  the entry point (src/main.rs): `mypi --server`, or the server a bare `mypi` becomes
        │
        ▼
      ipc ──► api ──► agent ──► storage
                │        │         │
                │        └─────────┴──► platform
                └──► config ──────────► platform
```

Nothing points back up. The guard table in `src/lib.rs` (`mod architecture`) is that rule as a
failing test.

### What the server holds in memory

The api's state (the writer, one session's live state per open session, the running rounds, the
relay, the config), the config itself, the time zone, the log ring, and the wire's own bits (the
database lock, the endpoint, the register of front ends). Everything else is the database or a file.

### Invariants

1. **One writer.** One thread ever writes the database, and one session runs one round at a time.
2. **The api is the only surface.** Nothing else under `server/` is a front end's business.
3. **One mouth per question.** Storage writes every row, config reads every file, platform asks the
   operating system, the loop runs rounds.
4. **The request side rebuilds from storage every lap**, so a restarted process sends the same bytes.
5. **The server renders nothing**, and a front end holds no server state.

## 2. Design intent

**One mouth per question.** The database, the files, the operating system, the loop and the front
end's surface are five problems with five rates of change; giving each one owner makes "where does
this belong" independent of who is asking.

**The direction rule is a test, not a convention.** The guard table in `src/lib.rs` fails a test when
a layer reaches the wrong way.

**The api is the only surface.** A front end learns one vocabulary, and the wire carries the api's
values — a second edge would only translate them.

**The loop rebuilds from storage.** Identical bytes across laps, restarts and resumes are what a
prefix cache is paid for.

**The server holds no conversation.** State has two homes — the database and the config — and what is
in memory is the residue neither can carry.

**The server stops when nobody is watching.** A front end's death is noticed by its connection, not
by a message, so nothing has to be timed out.

**Nothing in the server renders.** Keeping presentation out means the same content serves any front
end, and the server never has to know a screen's width.

## 3. Open questions

1. **No front end is wired yet.** `src/main.rs` connects and stops with a message (第 8 条).
2. **Windows.** The named pipe is unwritten and several places are unix-shaped (第 16 条, 第 40 条).
3. **Nothing reconnects** after the server dies (第 42 条), and **a front end cannot read the
   server's log** (第 41 条).
4. **The config is read once, at startup** — no hot reload yet (第 20 条).
5. **One-shot commands bypass the wire** (`mypi --sessions` opens its own read connection).

Each domain's own list (its README's §3) carries the rest.

## 4. Config

The server side reads `config.server.*`; `config.tui.*` belongs to the front end. This layer opens no
file; the sources and the naming rules are in `config/README.md`.

| area | what the server side takes from it |
|---|---|
| `config.server.api.*` | the default model |
| `config.server.request.*` | how much of a running round a front end sees; the compaction's tail and template |
| `config.server.agent.*` | the tool-round fence, the tool budgets and limits, the profile |
| `config.server.storage.*` | how many completed tool rounds land one batch |
| `config.server.platform.*` | the time zone, the stamp templates, the endpoint kind |
| `models.*` | providers, endpoints, keys, prices |
| `profile.*` | the enabled profile's system prompt, tool list and compaction instruction |

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
