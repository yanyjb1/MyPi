# config architecture

What the pieces are and how they relate. The design — the three areas, how a key is named, and the
invariants this layer keeps — is in `README.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations, `Config` itself, and the one load path: read both files, validate, write a bootstrap file when none exists |
| `paths.rs` | `ConfigPaths`: where the two files live (the `config.yaml` root, the `models.yml` beside it, the profile folder), the project root, and `absolute` |
| `read/config.rs` | the `config.` area: `ConfigFile` (the literal `config:` layer), `AppConfig` (`server` / `tui`), `ServerConfig` (`api` / `request` / `agent` / `storage` / `platform`), the per-area structs and their defaults, validation, the bootstrap writer, and `write_key` |
| `read/model.rs` | the `models.` area: `ModelsConfig`, `Provider`, `ModelEntry`, prices, the compatibility switches |
| `read/profile.rs` | the `profile.` area: finding the profile folder, listing names, resolving one (its system prompt, its compaction instruction, which tools it enables), and the active name |
| `prompts/` | the built-in texts, one file each, compiled into the binary with `include_str!` |

## The public surface

| item | what it does |
|---|---|
| `Config::load()` / `Config::load_from(paths)` | read both files through `ConfigPaths`, resolve every model and profile, validate, and write a fresh `config.yaml` when there is none |
| `Config::models()` | every model in `models.yml`, as `<provider>:<id>` |
| `Config::default_model()` | the configured default, resolved (provider, entry, key) |
| `Config::model_by_id(id)` | one model addressed `<provider>:<id>` |
| `Config::resolve_key(provider)` | the provider's API key, with `${ENV_VAR}` expanded through `platform::env` |
| `Config::set_default_model(id)` | write the default back into `config.yaml` (the `/model` path), and return the file it wrote |
| `ConfigPaths` | the roots, resolved to absolute paths when they are built |
| `AppConfig` and the per-area structs | the whole `config.` area, handed out as values |
| `read::model::{Provider, ModelEntry, …}`, `ResolvedModel` | the `models.` area, resolved |
| `read::profile::{resolve, list, active_name}` | the `profile.` area |
| `prompts::{builtin_system, compact_instruction, compact_context}` | the built-in texts |

Everything else in this folder is private or crate-internal.

## Relationships

- **Who calls config**: the api (the default model, the active profile's name, the whole `AppConfig`
  it resolves `Live.settings` from), and its own `paths` (whoever needs the profile folder). The
  agent and storage layers do not call it at all: they receive values, already resolved.
- **Who config calls**: `platform::env` (the home directory, the XDG roots, environment variables),
  `platform::os` (which family this is, so the bootstrap can pick its defaults), and the filesystem —
  the two files and the profile folder.
- **What it does not know**: the api, the agent, storage's schema, any front end. It reads files and
  hands out values; nothing else under `server/` opens a config file.
- **One reader per area**: `read/config.rs`, `read/model.rs`, `read/profile.rs`. A new key goes into
  the reader matching its area; there is no fourth place.

## Parts that must stay in step

1. A struct's field names and the file's keys: `serde` with `camelCase` and `default`, so a missing
   key falls back to the type's default and an unknown one is ignored.
2. `config.yaml`'s literal `config:` layer: the loader (`ConfigFile`) and the writer (`write_key`)
   must agree on it — a file missing the layer is an error, not a silent fallback.
3. `read/config.rs`'s doc headings (`` /// `config.server.api:` `` and the like) spell the key prefix
   out; they are the in-code statement of the naming rule the README sets.
4. `prompts/mod.rs`'s `include_str!` paths and the file names in `prompts/`.
5. `paths.rs`'s rule that a path is absolute from the moment `ConfigPaths` is built — the load path
   relies on it, and so does every later `absolute` call.
6. The bootstrap writer and the loader: the file the bootstrap writes must be one the loader reads
   back into the same shape.

## Not wired yet

1. `config.server.agent.tools.web.*` — the four output limits are declared, but the web engines still
   use their own constants (第 14 条).
2. `config.server.agent.reloadOnCompaction` — declared, and the semantics (re-read the profile's tool
   list after a compaction) are not implemented (`memo.md`, 会话从哪儿算起).
3. The process's working directory: `paths.rs` calls `std::env::current_dir()` itself instead of
   asking `platform` (第 22 条 — the rule that a built path never moves again has to survive that
   move).
4. `config.server.request.streaming` is honored at the backend-to-front-end edge
   (`src/ipc/stream.rs`): `immediate` / `buffered`. Whether the **gateway** answers with increments
   is a different question, and no key in this file answers it.
