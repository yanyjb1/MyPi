# server architecture

What the pieces are and how they relate; the design and its reasoning are in `README.md`, and each domain's own files and relationships are in its own `ARCHITECTURE.md`.

## Files

1. `mod.rs` — the declarations: the four domains.
2. `api/` — the front end's surface, in `api/ARCHITECTURE.md`.
3. `storage/` — the database and its writer, in `storage/ARCHITECTURE.md`.
4. `agent/` — the loop, the request half and the tools, in `agent/ARCHITECTURE.md`.
5. `config/` — the two files and the profiles, in `config/ARCHITECTURE.md`.

The operating system sits below this folder (`../platform/`), the wire (`../ipc/`) and the entry point (`../main.rs`) sit outside it, and the wire has its own pair (`ipc/README.md`, `ipc/ARCHITECTURE.md`).

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

- **Who calls what**: the entry point starts the wire, the wire calls the api's three doors, the api calls `agent` (one round), `storage` (the writer and the reader) and `config` (values), the agent calls `storage`, `config` and `platform`, and `storage` and `config` call `platform`.
- **Who reads the database**: everybody through `storage::Reader`, which owns every query and every decode, so nobody outside storage writes SQL or parses a stored shape.
- **Who reads a file**: `config`, and nothing else.
- **Who asks the operating system**: `platform`, and nothing else, while everyone asks it.
- **The contract**: the guard table in `src/lib.rs` (`mod architecture`) lists, per module prefix, what it must never reference, and a violation fails a test, so the layering is enforced rather than remembered.

## Threads

1. The storage writer — created by the api's state, lives until the process ends.
2. The live line — created by the api's state, lives until the process ends, with one queue in the order the loop sent.
3. A round or a compaction — created by the api, lives one round.
4. The wire's accept loop — created by `ipc::server::serve`, lives until the server stops.
5. One per connection — created by the accept loop, lives one connection, where a watch turns that thread into the stream's writer.
6. A provider subprocess — created by the transport when a connection has to be killed, lives one provider connection.

## Parts that must stay in step

1. The guard table and the module graph, so a file that moves keeps the rules naming its prefix.
2. The api's shapes and the wire, since `ipc`'s two enums name them and a new api shape crosses only if they do.
3. `config`'s key prefixes and its readers, so a new key goes into the reader matching its area.
4. `storage::decode`'s column lists and the block columns the agent's cut fills.
5. `platform`'s stamp shape, with one producer, one parser, and it is the shape the database stores.

## Not wired yet

1. Windows: the named pipe is unwritten and several places are unix-shaped (第 16 条, 第 40 条).

Each domain's own `ARCHITECTURE.md` carries its share of the rest.