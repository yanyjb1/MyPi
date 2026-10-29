# config

Every setting comes from one of three sources, and only this domain opens them: a layered
`config.yaml`, one global `models.yml`, and the `profiles/` folder.

## 1. Design

### Sources and layers

| source | location | layers |
|---|---|---|
| configuration | `config.yaml` | profile overlay > project root > global root > built-in default |
| model | `models.yml` | global root only; one file, never written |
| profile | `profiles/<name>/` | project root > global root |

A configuration key is resolved per key: the highest layer that has a valid value wins. A value
that does not parse, or an unknown key, produces a warning and the next layer is tried. A key that
no layer has takes the built-in default. `MYPI_CONFIG_DIR` replaces both roots with that one
directory.

### Platform

The global root is not built here: `platform::env::config_dir()` answers it. Unix uses
`$XDG_CONFIG_HOME/mypi`, else `~/.config/mypi`; Windows must answer `%APPDATA%\mypi`. The project
root is the process startup directory. `MYPI_CONFIG_DIR`, when set, is the only root: no project
search, no global search, and every source is read from under it. Paths are composed with `Path` and
`PathBuf`; no separator is written literally.

A profile is a directory under `profiles/`; its dotted relative path is its name. `system.md` is the
prompt, `tools.yaml` is an optional tool list, `compact.md` an optional compaction instruction, and
`config.yaml` an optional full configuration overlay. `default` is the default profile; `oh-my-pi`
is a second built-in name. Both built-ins use the compiled prompt.

The active profile is chosen by the session, not by a configuration key. The name is resolved
project-first; the same name in both roots uses the project copy and warns; the same name twice in
one root makes that name unusable; an unknown name is an error. A missing `default` falls back to
the compiled prompt.

### The key table

A key's name is `head.path.name`, all three stored separately. Heads: `config` for `config.yaml`,
`models` for `models.yml`. `path` mirrors the tree under the head; `name` is the leaf. The table is
the schema: `registry.rs` holds the types and lookup, `schema.rs` declares every key with its kind,
default and comment. `template()` renders the default `config.yaml` from that table.

`Config::get(key, profile)` returns the value for one key. Model keys name the full address:
`models.provider.<provider>.<field>`, `models.model.<provider>:<id>.<field>`. An unknown key is an
error, never a default.

### Bootstrap

When an artifact is missing, it is written next to `models.yml` in the global root: `config.yaml`
from the table, and `profiles/default/system.md` plus `compact.md` from the compiled prompts. Nothing
existing is overwritten. Other profiles are created by the user in the same layout.

### Warnings

A written key that is not used — unknown, wrong type, wrong enum or out of range — produces a
`ConfigNotice` and the next layer or default is used. A missing key does not warn. Only warning level
is produced.

### What lives in memory

One `Config` per process: the parsed layer trees and the model list. Effective values are built per
profile and cached. Disk stays the source of truth.

`Config::reload` re-reads both files with the roots this `Config` was built with; `MYPI_CONFIG_DIR`
is read by `Config::load` only and never again on reload. A failed reload leaves the old `Config`
alone; the caller keeps it.

## 2. Design intent

**Three sources, one reader each.** A value's origin decides who may edit it.

**The name carries its head.** `config.` and `models.` never share a namespace.

**A bad value never stops the program.** It is reported and replaced by the next valid layer.

**Profiles are presentation and behavior, not process state.** The profile overlay is the top layer;
the session carries which profile it uses.

## 3. Open questions

1. **Skills.** Deferred; the reader will mirror profiles (directory, name, description).
2. **`mcp`.** Removed from this domain; not a configuration source here.
3. **`config.server.agent.reloadOnCompaction`.** Declared, no semantics.

## 4. Config fields

This domain owns the keys below; consumers ask it for values.

| area | keys |
|---|---|
| `config.server.api` | `defaultModel` |
| `config.server.request` | `streaming`; `compact.retainTail`, `compact.instructionFile` |
| `config.server.agent` | `reloadOnCompaction`; `loop.maxToolCalls`; `tools.*`; `subagent.maxToolCalls` |
| `config.server.storage` | `batchTools` |
| `config.server.platform` | `timezone`, `stamp.*`, `ipc` |
| `config.server` | `pruneOnRewind` |
| `config.ui` | `session.preloadCache`, `placeholder.*` |
| `config.tui` | `theme.*`, `renderMargin`, `todoFold`, `usage.align`, `fx.usdCny` |
| `models.provider.<name>` | `baseUrl`, `api`, `apiKey`, `authHeader`, `models` |
| `models.model.<provider>:<id>` | `id`, `name`, `contextWindow`, `maxOutputTokens`, `currency`, `cost.*` |

`models.yml` top level is `providers` only. `baseUrl`, `api` and `models` are required; `api` has
no default. `currency` is `CNY` or `USD`; `cost` is per 1M tokens and all-zero means no quote.
