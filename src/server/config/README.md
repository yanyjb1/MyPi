# config

Every setting the user can change is read here and nowhere else: two files and a folder of profiles.
Nothing else under `server/` opens them.

## 1. The design

### Three areas

| area | source | holds |
|---|---|---|
| configuration | `config.yaml` | switches and numbers |
| model | `models.yml` | providers, models, prices, compatibility switches |
| profile | the `profiles/` folder | system prompt, compaction instruction, tool list |

`read/` mirrors that: one file per area — `read/config.rs`, `read/model.rs`, `read/profile.rs`. A new
key goes into the reader matching its area.

### Where the values come from

Roots, in order: the project root, then the global root. `MYPI_CONFIG_DIR`, when it is set, is the
only root. Each root may carry `config.yaml`, `models.yml` and `profiles/`, and the first root that
has a file is the one read. A root is absolute from the moment it is built.

`models.yml` is the user's file and the program never writes it. When no root has a `config.yaml`, a
fresh one is written from the built-in defaults.

### How a key is named

A key's name is its whole address: **area . path . name**.

| area | written as | controls | example |
|---|---|---|---|
| configuration | `config.` | everything `config.yaml` carries | `config.server.api.defaultModel` |
| model | `models.` | everything `models.yml` carries | `models.providers.<name>.baseUrl` |
| profile | `profile.` | what the enabled profile supplies | `profile.system` |

1. **area** — `config`, `models` or `profile`.
2. **path** — mirrors the `src/server` tree: `config.server.agent.loop`, `config.server.api`.
3. **name** — says what it is: `maxToolCalls`, `outputMaxBytes`.
4. All camelCase.
5. A section follows its reader: a key read only by the request layer lives under
   `config.server.request`.
6. One value, one entry, named after the layer that hands it out.
7. A UI section is named after the UI: `config.tui.*`. Placeholders hang under
   `config.tui.placeholder.*`, each leaf named after what it shows.
8. Where a key breaks one of these rules, the reason is stated at that key's own definition, in one
   line, and nowhere else.

### The profile area

`profile.*` names the **enabled** profile, resolved when it is asked for: `profile.system` is the
system prompt of whichever profile is enabled right now, and the other two values work the same way.
The area has no `path` segment and no sub-layers: three values, each named for what it is.

A profile is a directory under `profiles/`, and its dotted relative path is its name (`default`,
`work.rust`). `system.md` is the prompt; `tools.yaml`, when present, is the list of enabled tools;
`compact.md`, when present, overrides the compaction instruction. Two names are built in
(`oh-my-pi`, `default`).

### Invariants

1. Read here, and only here: nobody else opens the three sources.
2. A bad user value counts as absent: illegal input falls back to the built-in copy. Those copies
   live in `prompts/`, one file per prompt, compiled into the binary.
3. A numeric default sits next to the type that owns the value.
4. The compatibility rule table is compiled in and never read from disk.
5. `paths.rs` says where things are; everyone else asks it.

### What lives in memory

One loaded `Config` per process, built once at startup. A caller holds the copy it was handed.

## 2. Design intent

**Three areas, one reader each.** A value's origin decides who may edit it, and one reader per source
makes "where do I change this" one answer.

**The name carries its area.** A value can come from three places, so its name says which.

**A bad value counts as absent.** The program starts on a typo, and the built-in copy is the
documented behaviour.

## 3. Open questions

1. **Nothing re-reads the files at runtime.** A changed value needs a restart, and nothing downstream
   refreshes the lists it built from the old one (第 20 条).
2. **`config.server.agent.tools.web.*` has no reader.** The four output limits are declared; the web
   engines still use their own constants (第 14 条).
3. **`config.server.agent.reloadOnCompaction` has no semantics.** The key is declared; "re-read the
   profile's tool list after a compaction" is not implemented.
4. **The process's working directory is read from the environment, not from `platform`** (第 22 条).
5. **`config.server.request.streaming` is honored at the backend-to-front-end edge**
   (`src/ipc/stream.rs`). Whether the gateway itself answers with increments is a different question,
   and no key in this file answers it.

## 4. Config

This layer owns the three sources. Other domains ask it for values; none of them reads a file.

| source | root keys | file |
|---|---|---|
| configuration | `config.server.*` (api, request, agent, storage, platform) and `config.tui.*` | `config.yaml`, under each root |
| model | `models.providers.<name>.*` — where the provider is, its wire protocol, its key, its models' prices and switches | `models.yml`, under each root |
| profile | `profile.system`, `profile.tools`, `profile.compactInstruction` | `profiles/<name>/`, under each root |

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
