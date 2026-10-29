# config architecture

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and re-exports only |
| `registry/` | `types` (kinds, defaults, values, `Key`), `macros` (the `keys!` macro), `table` (lookup, predicates, key helpers, `template`) |
| `schema.rs` | the one `keys!` invocation: every `config.*` and `models.*` key with kind, default and comment |
| `load/` | `types` (`Config`, `Inner`, `Layers`, `Effective`), `entry` (load/reload), `api` (default model, profiles, writes), `source` (layer reading, bootstrap), `effective` (per-key merge), `check` (merged-value checks) |
| `model/` | `types` (providers, models, prices, currency), `parse` (the file and providers), `entry` (one model), `query` (keyed lookup) |
| `profile/` | `types` (`ResolvedProfile`), `discover` (scan, precedence, duplicate names), `resolve` (prompts, tools, compaction) |
| `paths.rs` | `ConfigPaths`: project and global roots, the files and folders under them |
| `notice.rs` | `ConfigNotice` and the shared warning constructor |
| `write.rs` | textual `write_key` for `config.yaml` |
| `prompts/` | compiled prompts: `omp_system`, `compact_instruction`, `compact_context` |

## Public surface

- `Config::load`, `Config::load_from(ConfigPaths)`, `Config::reload()`
- `Config::get(key, profile)`, `Config::get_json(key, profile)` — one value; unknown key is an error
- `Config::effective(profile)` — all `config.*` values for one profile
- `Config::models`, `model_by_id`, `default_model(profile)`, `resolve_key(provider)`
- `Config::resolve_profile(name)`, `Config::notices(profile)`, `Config::set_default_model(id)`
- `Config::paths`
- `Value`, `ConfigNotice`, `ResolvedProfile`
- `ModelsConfig`, `Provider`, `ModelEntry`, `Cost`, `Currency`, `ResolvedModel`
- `paths::ConfigPaths`
- `KEY_*` constants from `schema.rs`; `registry::{lookup, template, model_key, provider_key}`
- `prompts::{omp_system, compact_instruction, compact_context}`

## Relationships

- **Who calls config**: the api (`Call::Config`, default model, pools), the agent register
  (`resolve_profile`, models, default model), and the web tools (browser settings through values).
- **Who config calls**: `platform::env` — the environment and the global config root; its
  `config_dir()` must answer the platform's native directory (`%APPDATA%\mypi` on Windows) —
  `platform::os` indirectly through it, and `agent::request::protocols::ProtocolId` to validate a
  provider's `api`.
- **What it does not know**: the api, storage's schema, any front end, `mcp`. It reads three sources
  and hands out values.

## Must stay in step

1. `schema.rs` and `registry::template()`: a key exists in exactly one place, and the template is
   rendered from the table.
2. `profile.rs`'s file names (`system.md`, `tools.yaml`, `compact.md`, `config.yaml`) and
   `paths.rs`'s constants.
3. `write.rs`'s key path `config.server.api.defaultModel` and the schema entry.
4. `load.rs`'s bootstrap prompt sources and `prompts/`'s compiled texts.
5. The README's key table and `schema.rs`.

## Not wired yet

1. `src/server/api/call/config.rs` still answers `Refuse::Internal`; it needs `Call::Config` to carry
   an optional session so the profile layer can be resolved.
2. Warnings are collected but not forwarded through the notice channel.
3. External call sites still use the old typed `cfg.app.*`, `McpConfig` and no-argument
   `default_model`; they move when their own domains are refactored.
4. Skills are not read here yet.
