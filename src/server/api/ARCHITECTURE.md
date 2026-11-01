# api — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the three exports |
| `api.rs` | `Api`, `Conn`, `Refuse`: the connection table, the frame entry, the booking of an existing session |
| `route.rs` | the name tables: kind plus name pick a door; JSON picks its arguments |
| `relay.rs` | the stream: viewers per session, the running turn, the buffer, the frame shapes |
| `convert.rs` | the rewind cut, the only storage shape this domain renames |
| `call/` | the read doors: `init`, `blocks`, `meta`, `usage`, `todo`, `memo`, `children`, `config`, `pools`, `sessions` |
| `post/` | the write doors: `init`, `name`, `cwd`, `switch`, `delete`, `interrupt`, `rewind`, `default_model`, `reload`; `emit.rs` holds the writer gate and the change event |
| `subscribe/` | `watch`, `submit`, `compact` |

## What it talks to

- **`agent::r#loop`**: `Loop::{book, booked, deliver, interrupt, drop, change, mailbox, todo}`,
  `Birth`, `Message`, `Change`, `Retention`, and the live line `Event`.
- **`agent::request`**: `Request::new(Options)` once, at startup; `Options` carries the providers,
  the retry policy, the per-address output caps, the streaming mode and the compaction tail.
- **`agent::tools`**: `registry::{names, roster}` when `init` writes the archive; `web::close_session`
  when a session is deleted, under the `web` feature.
- **`storage`**: `Writer` for every row and every deletion, `Reader` for every read,
  `query::{Sheet, Cond, Order, Query, Val}` for the filtered reads, `Structure` as the relay's
  second source.
- **`config`**: `Config`, read once at startup; `resolve_profile`, `default_model`, `models`,
  `profiles`, `get_json`, `set_default_model`, and the loop/request values read while `Api::open`
  builds them.
- **Who calls it**: `ipc::server` — `Api::open`, `connect`, `receive`, `disconnect`, `idle_for`.
- **Knows nothing about**: the front end, the terminal, SQL, column names, block bodies.

## Inside

```text
Api::receive
  → register  ── hello / bye
  → call      ── route::call   → call/*  ── Reader
  → post      ── route::post   → post/*  ── Writer, Loop, Config
  → subscribe ── route::subscribe → watch | submit | compact ── Relay, Loop
  → reply frame, or the error frame

Relay::start
  ← storage Writer relay port      (Structure: session, turn, ranges)
  → Sender<Event> handed to Loop::new
```

- `Api::open` builds the request half and the loop, in that order, so the loop's events already have
  a dispatcher when the first turn opens.
- `Api::ensure_booked` puts a session the loop has never seen back on the table: it reads the
  session row and its archive, takes the tool names out of the archive's manuals, and books.
- `submit` and `compact` share `submit::open`: the busy check, the booking, the delivery, and the
  writer slot.

## Must stay in step

1. `route.rs` and the `call/` `post/` `subscribe/` files: a door is a name here and one function there.
2. `relay.rs`'s frame shapes and `README.md`'s tables: the front end reads one and this writes it.
3. `post/init.rs`'s archive and `Loop`'s request assembly: both expand the same names through
   `registry::roster`, so a session's stored manuals and its live manuals are the same bytes.
4. `Api::open`'s `Options` and `Config`'s accessors: a value added to one needs its key in the other.
5. `post/delete.rs` and `relay::forget`: a session leaves the table, the relay and the database.

## Not wired

1. The front end still imports the retired `ipc::contract` shapes; the `ui` tree does not compile.
2. `usage` quotes only what `models.yml` declares; a model with no price answers `null`.
