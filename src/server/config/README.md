# config

Every setting comes from one of three sources, and only this domain opens them: a layered `config.yaml`, one global `models.yml`, and the `profiles/` folder.

## 1. Design

### Sources and layers

1. Configuration — `config.yaml`, layered as profile overlay > the project's `.mypi` > global root > built-in default.
2. Model — `models.yml` in the global root only, one file, never written.
3. Profile — `profiles/<name>/`, layered as the project's `.mypi` > global root.

A configuration key is resolved per key, and the highest layer that has a valid value wins.
A value that does not parse, or an unknown key, produces a warning and the next layer is tried, and a key no layer has takes the built-in default.
`MYPI_CONFIG_DIR` replaces both roots with that one directory.

### Platform

The global root is not built here, because `platform::env::config_dir()` answers it: unix uses `$XDG_CONFIG_HOME/mypi`, else `~/.config/mypi`, and Windows must answer `%APPDATA%\mypi`.
`config.server.platform.home` names what `~` stands for across the whole program, where empty means this platform's own home; the entry point hands it to `platform::env` once at startup, and the api answers the key resolved, so a front end asks the same question instead of reading the environment.

The project root is the process startup directory, and it counts only when it carries `.mypi`.
The project layer lives inside that folder, as `<startup>/.mypi/config.yaml` and `<startup>/.mypi/profiles/`, so nothing this tool writes or reads scatters beside a project's own files.
`MYPI_CONFIG_DIR`, when set, is the only root, with no project search, no global search, and every source read from under it; paths are composed with `Path` and `PathBuf`, and no separator is written literally.

A profile is looked up for the directory the session stands in rather than for the process: a session opened in `A/` reads `A/.mypi/profiles/`, one that opened in `B/` reads `B/.mypi/profiles/`.
Only that directory is asked, no parent is searched, and since a profile is picked before the session opens, a `cd` during a session moves neither its prompt nor its tools but only what the next session may open with; a directory without `.mypi` has no project layer at all.

A profile is a directory under `profiles/`, and its dotted relative path is its name: `system.md` is the prompt, `tools.yaml` is an optional tool list, `compact.md` an optional compaction instruction, and `config.yaml` an optional full configuration overlay.
`default` and `ENGINEER` are built in, each carrying a compiled prompt and a compiled compaction instruction, and running on the full tool table unless a directory of that name supplies its own.
The active profile is chosen by the session rather than by a configuration key: the name is resolved project-first, the same name in both roots uses the project copy and warns, the same name twice in one root makes that name unusable, an unknown name is an error, and a built-in name with no directory falls back to its compiled prompt.

### The key table

A key's name is `head.path.name`, with all three stored separately, where the heads are `config` for `config.yaml` and `models` for `models.yml`, `path` mirrors the tree under the head, and `name` is the leaf.
The table is the schema: `registry.rs` holds the types and lookup, `schema.rs` declares every key with its kind, default and comment, and `template()` renders the default `config.yaml` from that table.
`Config::get(key, profile)` returns the value for one key, model keys name the full address as `models.provider.<provider>.<field>` or `models.model.<provider>:<id>.<field>`, and an unknown key is an error rather than a default.

### Bootstrap

When an artifact is missing, it is written next to `models.yml` in the global root: `config.yaml` from the table, one directory per built-in profile with `system.md` and `compact.md` from the compiled texts, and, in a build with the `browser` feature, the site-rule manifest `web-rules.json` plus every `web-rulesets/` file it names.
Nothing existing is overwritten, and a directory of a profile's name in any root stops that one write, because the profile list is read per name and one built-in profile present never hides another.
A write that fails is a notice rather than a failure, since the compiled copy answers instead, and other profiles are created by the user in the same layout.

### Site rules

`web-rules.json` is a manifest: one array whose entries are either a rule written in place or a string naming another file of rules, resolved against the manifest's own directory and written with forward slashes.
Each set is flattened in where it is named, so one ordered list of rules emerges.
The built-in manifest names one set per site or site family under `web-rulesets/`, and a read that warns loses only that set.
The browser domain is the reader: it asks for the regions a page keeps, and for the drops and plaintify lists inside each.

### Warnings

A written key that is not used, whether unknown, wrong type, wrong enum or out of range, produces a `ConfigNotice` and the next layer or default is used.
A missing key does not warn, and only the warning level is produced.

### What lives in memory

One `Config` per process holds the parsed layer trees and the model list, effective values are built per profile and cached, and disk stays the source of truth.
`Config::reload` re-reads both files with the roots this `Config` was built with, `MYPI_CONFIG_DIR` is read by `Config::load` only and never again on reload, and a failed reload leaves the old `Config` alone for the caller to keep.

## 2. Design intent

- **Three sources, one reader each.** A value's origin decides who may edit it.
- **The name carries its head.** `config.` and `models.` never share a namespace.
- **A bad value never stops the program.** It is reported and replaced by the next valid layer.
- **Profiles are presentation and behaviour, not process state.** The profile overlay is the top layer, and the session carries which profile it uses.

## 3. Open questions

1. **Skills.** Deferred, and the reader will mirror profiles (directory, name, description).
2. **`mcp`.** Removed from this domain, and not a configuration source here.

## 4. Config fields

This domain owns the keys below, and consumers ask it for values.

1. `config.server.api` — `defaultModel`.
2. `config.server.request` — `streaming`, and `compact.retainTail` with `compact.instructionFile`.
3. `config.server.agent` — `loop.maxToolCalls`, `loop.scrooge`, `loop.tools.task.timeoutSecs`, `loop.tools.memoFolder`, `tools.*` and `subagent.maxToolCalls`.
4. `config.server.storage` — `batchTools`.
5. `config.server.platform` — `timezone`, `stamp.*` and `ipc`.
6. `config.server` — `pruneOnRewind`.
7. `config.ui` — `lang`, `session.preloadCache` and `placeholder.*`.
8. `config.ui.tui` — `fx.usdCny`.
9. `config.ui.tui.zone.main.history` — `cacheMargin`, `focusLine`, `cardFoldBudget`, `scrollHeight` and `todoFold`.
10. `models.provider.<name>` — `baseUrl`, `api`, `apiKey`, `authHeader` and `models`.
11. `models.model.<provider>:<id>` — `id`, `name`, `contextWindow`, `maxOutputTokens`, `vision`, `subagent`, `reasoningEfforts`, `currency` and `cost.*`.

`models.yml` has `providers` at its top level and nothing else, `baseUrl`, `api` and `models` are required, and `api` has no default.
`currency` is `CNY` or `USD`, and `cost` is per 1M tokens where all-zero means no quote.
`vision` says the model takes image parts, `reasoningEfforts` lists the thinking-effort rungs it takes with an absent list meaning none, and `subagent` says the `task` tool may offer the model to a child, absent being false while a session's own model is offered regardless.