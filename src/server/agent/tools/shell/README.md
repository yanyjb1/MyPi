# shell

`shell` and `alias`: the tools that reach a command line.

## 1. Design / Schema

### Files

| file | holds |
|---|---|
| `tool.rs` | the `shell` tool |
| `admit.rs` | `admit`: aliases → `#ref` → guard, and the line it hands the runner |
| `segment.rs` | the separator set and the segment heads; the guard and the alias expansion share both |
| `guard.rs` | the command classifier: `normalize`, `classify`, `Verdict`, `RuleHit` |
| `runner.rs` | one command line: `run`, `run_to`, `OutputSink`, `Exit`, `StopFlag`, `readable`; hands on every byte already readable |
| `alias/tool.rs` | `alias` add/list/remove; the config layer is read-only here |

### One command

```text
admit:  expand_alias  →  rewrite #<ref>  →  guard::classify     then  runner::run
```

`admit` is the only way raw text becomes a spawnable line; `shell` and `kairos` both call it. `guard` sees the final command. It returns `Allow`, `Warn(ids)` or `Block(ids)`; a blocked command never starts. `kairos` admits at tool-call time and freezes the admitted line, so a job exists only for a command that already passed. `alias add` runs the same classifier on the command it stores.

One call may carry several lines; the shell runs them in order. A bare newline is a separator to the guard, a backslash before a newline is a continuation, and `expand_alias` rewrites the head word of every segment once.

The command itself runs in the session's shell (`Env.shell.backend`): `bash` or `zsh` on unix, `pwsh` or `niu` on windows. `platform::shell` picks the backend, builds the command line, sets how each backend emits UTF-8, and writes the quoted path a `#<ref>` becomes; `platform::proc` starts it as a tree of its own and stops it as a whole; the platform picks the ladder. A stop arrives through the `Stop` the runner registered after the spawn; the tool thread itself only receives, so nothing wakes it on a timer. Stdin is the null device: a command that prompts reads EOF, never the terminal the front end owns.

### shell

| field | value |
|---|---|
| `command` | always; one or more lines |

`shell` takes no array. A command's own deadline is the session's `shellTimeout`; a command that runs past it reports `timed out after Ns` through `ToolOutput::failure`, which is not a stop.

### alias

| field | value |
|---|---|
| `type` | `add`, `list`, `del` |
| `aliases` | `add`: one item per alias, with `name` and `command` |
| `names` | `del`: the names to remove |

`alias` expands the head word of every command segment, once, and adds no variables, functions or multi-line bodies. A name a session writes is one word the scanner accepts as a segment head; a name the config fixes is read-only for the session. `alias add` runs the guard on the command it stores.

`add` takes an array of aliases; `del` takes `names` as a text list; `list` is a query operation and refuses an array. `list` answers JSON, and the same value rides along as the memo.

## 2. Design intent

- **One gate for every command.** The guard judges the text that actually runs, after aliases and `#ref` are resolved, so no path around it exists.
- **A guard, not a sandbox.** The model runs real shell commands; the classifier blocks the machine-destroying shapes and warns on the rest, and it always judges the expanded command.
- **One backend, one shape.** `platform::shell` owns the program, the argument shape and the encoding; the tool owns the text it says.
- **UTF-8 everywhere.** All three backends emit UTF-8: `niu` natively, the unix shells via `LC_CTYPE`, `pwsh` via `[Console]::OutputEncoding`; a failed setting never refuses the command, and a native program that writes GBK (an old console tool on Chinese Windows) reads as garbled.
- **One cwd.** `cd` writes the handle every tool reads; see `cd/README.md`.

## 3. Unresolved

1. The guard belongs to the unix shells only; `pwsh` and `niu` run the line as written.
2. On unix the backend sets `LC_CTYPE`; a `LC_ALL` already in the environment still wins, so a shell launched under a forced `C` locale can emit the old bytes.

## 4. Config fields

| key | meaning |
|---|---|
| `config.server.agent.tools.shell` | backend: `bash`, `zsh`, `pwsh` or `niu`; detected once at first start and written here; a named one must be on `PATH` |
| `config.server.agent.tools.shellTimeout` | one shell command's deadline; default 600 s |
| `config.server.agent.tools.aliases` | user aliases as `name=command`; every session reads them, nothing shadows them |

The backend is detected once at first start and written back here; every later start reads the name.
