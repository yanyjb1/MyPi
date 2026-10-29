# server architecture

What the pieces are and how they relate. The design and its reasoning are in `README.md`. Each
domain's own files and relationships are in its own `ARCHITECTURE.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations: the five domains |
| `api/` | the front end's surface — `api/ARCHITECTURE.md` |
| `storage/` | the database and its writer — `storage/ARCHITECTURE.md` |
| `agent/` | the loop, the request half, the tools — `agent/ARCHITECTURE.md` |
| `config/` | the two files and the profiles — `config/ARCHITECTURE.md` |
| `platform/` | the operating system — `platform/ARCHITECTURE.md` |

The wire (`../ipc/`) and the entry point (`../main.rs`) sit outside this folder; the wire has its own
pair (`ipc/README.md`, `ipc/ARCHITECTURE.md`).

## Relationships

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

- **Who calls what**: the entry point starts the wire; the wire calls the api's three doors; the api
  calls `agent` (one round), `storage` (the writer, and the reader) and `config` (values); the agent
  calls `storage`, `config` and `platform`; `storage` and `config` call `platform`.
- **Who reads the database**: everybody through `storage::Reader`, which owns every query and every
  decode; nobody outside storage writes SQL or parses a stored shape.
- **Who reads a file**: `config`, and nothing else.
- **Who asks the operating system**: `platform`, and nothing else — everyone asks it.
- **The contract**: the guard table in `src/lib.rs` (`mod architecture`) lists, per module prefix,
  what it must never reference. A violation fails a test, so the layering is enforced rather than
  remembered.

## Threads

| thread | created by | lives |
|---|---|---|
| the storage writer | the api's state | until the process ends |
| the relay | the api's state | until the process ends; hands storage's pieces to the watchers |
| a round / a compaction | the api | one round |
| the wire's accept loop | `ipc::server::serve` | until the server stops |
| one per connection | the accept loop | one connection; a watch turns that thread into the stream's writer |
| a provider subprocess | the transport, when a connection has to be killed | one provider connection |

## Parts that must stay in step

1. The guard table and the module graph — a file that moves keeps the rules naming its prefix.
2. The api's shapes and the wire: `ipc`'s two enums name them, so a new api shape crosses only if
   they do.
3. `config`'s key prefixes and its readers — a new key goes into the reader matching its area.
4. `storage::decode`'s column lists and the block columns the agent's cut fills.
5. `platform`'s stamp shape: one producer, one parser, and it is the shape the database stores.

## Not wired yet

1. The front end: `src/main.rs` connects and stops with a message (第 8 条).
2. Windows: the named pipe is unwritten and several places are unix-shaped (第 16 条, 第 40 条).

Each domain's own `ARCHITECTURE.md` carries its share of the rest.
