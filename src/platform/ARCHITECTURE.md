# platform architecture

What the pieces are and how they relate; the design — what this layer is for, and the promises it keeps — is in `README.md`.

## Files

- `mod.rs` — the declarations.
- `os.rs` — which system this is: `Family`, `Os` (family, name, architecture) and `current()`, decided at compile time with no probing and no subprocess.
- `env.rs` — the directories and the environment variables behind them: home, data directory, config directory, the system temp directory, one raw variable, and the one anchor `~` is expanded against (`anchor`, `tilde_home`, `expand`).
- `time.rs` — the process's fixed offset, and the program's one stamp shape: `init`, `display`, `now_stamp`, `age_seconds`, `days_from_stamp`.
- `stamp.rs` — the timestamp templates: `parse` and `render` turn two instants into the prefix and suffix of one user message.
- `log.rs` — the in-process ring of what happened to us: a level, a scope, a message and a millisecond.
- `shell.rs` — which shell runs a command line: `Kind`, `Shell`, `detect`, `resolve`, `names`, the one-shot argument shape, and the quoted path a stored text becomes.
- `proc.rs` — one command's process tree: `Process` (`spawn`, `spawn_detached`, `id`, the three `take_*`, `stop`, `try_wait`, `wait`, `wait_timeout`), the single `process-fork` creation thread and the per-tree reaper, with the containers in `proc/sys_unix.rs` and `proc/sys_windows.rs`.
- `stop.rs` — the stop signal: `Stop` (flag plus a table of actions, each run on its own thread) and `Hook` (a registration that comes off on drop), with `new`, `is_stopped`, `set`, `reset` and `on_stop`.

## The public surface

- `os::{Family, Os, current()}` — which family, name and architecture this is.
- `env::{home, data_dir, config_dir, temp_dir, downloads_dir}` — the directories, XDG-based on unix and `%APPDATA%`/`%LOCALAPPDATA%` on windows, where `downloads_dir` reads `XDG_DOWNLOAD_DIR` and falls back to `~/Downloads`, and the IPC endpoint directory belongs to `ipc::endpoint`.
- `env::{var, var_os}` — one environment variable, as text or raw.
- `env::{anchor, tilde_home, expand}` — what `~` stands for, fixed once at startup from the configuration, and the one place a leading `~` is expanded.
- `time::init(name)` — take the offset from configuration, once, at startup (`None` or a geographic name means the machine's).
- `time::now_stamp()` — now, in the process's offset, as `YYYY-MM-DD HH:MM:SS`.
- `time::age_seconds(stamp)` — how old a stamp is.
- `time::days_from_stamp(stamp)` — a stamp's day count since 1970-01-01, for weekdays.
- `stamp::{Parts, parse, weekday, render, inject}` — the injection templates: render a template against two instants, or both halves of one message at once.
- `log::{Level, Record, push, info, warn, error, recent, by_scope, len}` — the ring.
- `shell::{Kind, Shell, names}` — the session's shell, where `resolve` refuses a shell the platform does not run or cannot find on `PATH`.
- `proc::{Process, Origin}` — start a following or detached tree, take its streams, stop it, wait for it.

Everything else in this folder is private.

## Relationships

1. **Who asks platform**: any layer, because it stands under all of them, and in practice config (the directory roots, the environment and the family for its bootstrap), the api (the clock when it opens a session or lands a message, the log, and the data directory the database file sits in), the agent (the clock when a block lands, the log, the temp directory the artifact files are materialized under, plus the shell and the process tree the `shell` tool runs on), web (the process tree the browser is launched into, and the stop it waits on), the bridge (the process tree its adapters are started in) and `git` (the process tree one git call runs in).
2. **Who platform asks**: nobody of ours, only the standard library and the operating system, and the guard table in `src/lib.rs` keeps it that way by putting this folder in no other layer's forbidden list while its own names every one of them.
3. **One mouth for the environment**: `env.rs` is the only file that reads environment variables and the only place a directory is decided, `time.rs` is the only file that reads zoneinfo, and `log.rs` holds the process's one ring.
4. **One process, one offset**: the offset is taken once from the configuration or the machine and never changes, and the ring and the offset are the two things this layer keeps in memory.
5. **Storage asks it nothing**: every instant arrives as a value, and the caller reads the clock.

## Parts that must stay in step

1. The stamp shape `YYYY-MM-DD HH:MM:SS`, in local time with no offset suffix, with one producer (`time::now_stamp`), one parser (`stamp::parse`), and it is the shape storage stores.
2. The two template syntaxes in `stamp::render` (the letter set and the strftime set) and the arithmetic in `time`, which are the same instants spelled two ways.
3. `os::current()` and the per-family rules `env.rs` applies: the directory names on unix, and the fallback everywhere else.
4. The log's record shape (level, scope, message, milliseconds), so whoever reads the ring, including a front end, sees exactly these fields.
5. `time::init` and the lazy fallback in `time.rs`: if nobody initializes the offset the machine's own wins, so a process that wants the configured one has to call `init` before its first stamp.

## Not wired yet

1. **The log ring has no reader outside the process.** `recent` and `by_scope` have no caller, so what this layer narrates is only visible while a debugger is attached, and a front end would need a door of its own (`src/ipc/README.md`).
2. **`stamp::inject` has no caller.** The timestamp injection happens when a user message is written, and where exactly that is stays undecided until the instant-messaging mode gets its own design (`memo.md`), which is why `config.server.platform.stamp.*` has no reader either.
3. **Windows.** `env.rs`, `shell.rs`, `proc.rs` and `time.rs` carry both families, but two places still assume unix: the browser paths, and the parent-death guarantee below the leader, where a job object covers the whole tree on windows while unix only covers the leader (第 16 条 lists the sites).