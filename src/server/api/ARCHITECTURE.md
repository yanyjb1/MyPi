# api ARCHITECTURE

## Files

| Path | Holds |
|---|---|
| `mod.rs` | `Api { cfg }`, `receive`, `Response`. The only `(kind, name)` router. |
| `route.rs` | The name table: payload decode (`bad_payload`), one call per name, error frames. |
| `register.rs` | `hello` / `bye` → `Silence`; no state. |
| `call/mod.rs` | Ten read functions; opens a `Ctx` (config, reader, attached session, sessions, running). |
| `call/ctx.rs` | The read context struct. |
| `call/session/` | `init` (structure + newest blocks), `blocks`, `meta`, `usage`; shared read helpers. |
| `call/tool/` | `todo`, `memo`, `children`. |
| `call/server/` | `config`, `pools`, `sessions`. |
| `post/mod.rs` | Thirteen write functions; busy checks for `delete`, `rewind`, `reload`. |
| `post/init.rs` | Session creation and profile selection. |
| `post/session/` | `name` `cwd` `switch` `profile` `todo` `delete` `interrupt` `rewind` `unwatch`. |
| `post/server/` | `default_model`, `reload`. |
| `post/tool/` | `memo` add/mod/del. |
| `subscribe/mod.rs` | The three stream entries; each builds a channel and attaches it. |
| `subscribe/watch.rs` | Attach as Watcher to the whole stream. |
| `subscribe/submit.rs` / `compact.rs` | Open a round through the driver, attach as Writer on its turn. |
| `notice.rs` | `notify(session, level, content)`: a notice frame into the session's stream. |
| `convert.rs` | storage/agent shapes → wire shapes (block, session meta, todo phase, turn usage). |
| `driver/mod.rs` | The global `STATE` lock, `Driver` (cfg, writer, sessions, running, relay), submit/compact/rewind/interrupt. |
| `driver/subscriptions.rs` | The stream table: `SessionStream`, `Subscriber`, turn buffer, `deliver`, frame builders. |
| `driver/relay.rs` | storage's port; pieces → stream frames; the methods the driver calls. |
| `driver/round.rs` | One round on its thread; `Preview` outlet for tool output. |
| `driver/compact.rs` | The summarization thread. |
| `driver/wake.rs` | The wake worker: watchless wakes are dropped, busy ones queued. |
| `driver/sessions.rs` | Live sessions, MCP bridge, wake queues. |
| `driver/running.rs` | The per-session round claim flags. |

## Relations

- `ipc::server::handle` calls `Api::receive` and acts on `Response`. It knows no names.
- `route.rs` names `call::*`, `post::*`, and `subscribe::*` functions; those own no state and
  reach the driver through `driver::state(cfg)`, the one global lock.
- `driver/relay.rs` delivers every outbound frame through `driver/subscriptions.rs`; both the
  relay thread (storage pieces) and the doors go through the same `deliver`.
- `driver` may use `crate::ipc::contract` shapes; `call`/`post`/`subscribe` handlers may not
  name transport code.
- `notice::notify` is called by other server domains (agent tools) when they have something
  to say about a session.
- `convert.rs` must stay in sync with `storage`'s row shapes and `contract/data.rs`.

## Not yet wired

- `ui::session`, `ui::tui`, and `oneshot.rs` still name the old api doors (`Api::call`,
  `Api::post`, `Api::watch`); they must move to `Api::receive` through the ipc client.
- `driver/round.rs`'s `Preview` outlet only forwards `tool_output`; retry and budget events
  are logged or ignored as before this wave.
