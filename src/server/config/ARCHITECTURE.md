# config architecture

## Files

1. `mod.rs` — module declarations and re-exports only.
2. `registry/` — `types` (kinds, defaults, values, `Key`), `macros` (the `keys!` macro), and `table` (lookup, predicates, key helpers, `template`).
3. `schema.rs` — the one `keys!` invocation: every `config.*` and `models.*` key with kind, default and comment.
4. `load/` — `types` (`Config`, `Inner`, `Layers`, `Effective`, `BrowserOptions`), `entry` (load and reload), `api` (default model, profiles, writes), `source` (layer reading, bootstrap), `effective` (per-key merge), `check` (merged-value checks).
5. `model/` — `types` (providers, models, prices, currency), `protocol` (`ProtocolId` and its error), `effort`, `parse` (the file and providers), `entry` (one model), `query` (keyed lookup).
6. `profile/` — `types` (`ResolvedProfile`, the built-in table), `discover` (scan, precedence, duplicate names, the names on offer), `resolve` (prompts, tools, compaction).
7. `paths.rs` — `ConfigPaths`: the project's `.mypi` and the global root, the files and folders under them, and `project_dir`.
8. `notice.rs` — `ConfigNotice` and the shared warning constructor.
9. `rules.rs` — the site-rule manifest, the sets it names, their installation and flattening (feature `browser`).
10. `web-rulesets/` — one set per site or site family: the rules the built-in manifest names.
11. `write.rs` — textual `write_key` for `config.yaml`.
12. `prompts/` — one folder per built-in profile, each holding `SYSTEM.md` and `COMPACTION.md`.

## Public surface

- `Config::load`, `Config::load_from(ConfigPaths)`, `Config::reload()`.
- `Config::get(key, profile)` and `Config::get_json(key, profile)`: one value, where an unknown key is an error.
- `Config::effective(profile)`: all `config.*` values for one profile at the process's roots.
- `Config::effective_in(cwd, profile)`: the same at one directory's roots.
- `Config::models`, `model_by_id`, `default_model(profile, cwd)`, `resolve_key(provider)`.
- `Config::resolve_profile(name, cwd)`, `Config::profiles(cwd)`, `Config::notices(profile)`, `Config::set_default_model(id)`.
- `Config::paths`.
- `Value`, `ConfigNotice`, `ResolvedProfile`.
- `ModelsConfig`, `Provider`, `ModelEntry`, `Cost`, `Currency`, `ResolvedModel`.
- `ProtocolId`, `ProtocolError`, `Effort`: the `api` and `effort` values of `models.yml`.
- `paths::ConfigPaths`.
- `KEY_*` constants from `schema.rs`, and `registry::{lookup, template, model_key, provider_key}`.
- `prompts::{system, compaction, engineer, engineer_compaction}`.
- `rules::load(paths, notices)` for the site rules in force, and `rules::install(root)` for the manifest and the sets it names written where the user can edit them; `rules::{Rule, Part}` carry no fields to their readers, since `Rules::regions`, `Part::selector` and `Part::drops` are the whole surface.

## Relationships

- **Who calls config**: the api (`Call::Config`, default model, pools), the agent register (`resolve_profile`, models, default model), and the browser tools, which receive `BrowserOptions`, `Rules` and the bring-to-front switch as values.
- **One directory's roots**: a profile is resolved for the directory a session stands in, so `effective_in`, `resolve_profile`, `profiles` and `default_model` take a `cwd`; the project layer and its profile list are read once per project folder and kept, and nothing caches "the current project".
- **Who config calls**: `platform::env` for the environment and the global config root, whose `config_dir()` must answer the platform's native directory (`%APPDATA%\mypi` on Windows), `platform::os` indirectly through it, and `ipc::endpoint` for the endpoint kind.
- **What it does not know**: the api, storage's schema, any front end, `mcp`; it reads three sources and hands out values.

## Must stay in step

1. `schema.rs` and `registry::template()`, because a key exists in exactly one place and the template is rendered from the table.
2. The file names in `profile.rs` (`system.md`, `tools.yaml`, `compact.md`, `config.yaml`) and the constants in `paths.rs`.
3. The key path `config.server.api.defaultModel` in `write.rs` and the schema entry.
4. The bootstrap seeding one directory per entry of `profile/`'s built-in table, from the compiled texts in `prompts/`.
5. The README's key table and `schema.rs`.
6. `Provider.api` and `ProtocolId`, because the file spells the protocol as text and the parsed provider carries the value.

## Not wired yet

1. External call sites still use the old typed `cfg.app.*`, `McpConfig` and no-argument `default_model`, and they move when their own domains are refactored.
2. Skills are not read here yet.