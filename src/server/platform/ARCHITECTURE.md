# platform architecture

What the pieces are and how they relate. The design — what this layer is for, and the promises it
keeps — is in `README.md`.

---

## Files

| file | holds |
|---|---|
| `mod.rs` | the declarations |
| `os.rs` | which system this is: `Family`, `Os` (family, name, architecture) and `current()` — decided at compile time, no probing, no subprocess |
| `env.rs` | the directories and the environment variables behind them: home, data directory, config directory, the system temp directory, and one raw variable |
| `time.rs` | the process's time zone, and the program's one stamp shape: `init`, `now_stamp`, `age_seconds`, `days_from_stamp` |
| `stamp.rs` | the timestamp templates: `parse` and `render` turn two instants into the prefix and suffix of one user message |
| `log.rs` | the in-process ring of what happened to us: a level, a scope, a message and a millisecond |

## The public surface

| item | what it does |
|---|---|
| `os::{Family, Os, current()}` | which family, name and architecture this is |
| `env::{home, data_dir, config_dir, temp_dir, runtime_dir}` | the directories, XDG-based on unix (`runtime_dir` is where the ipc endpoint lives) |
| `env::{var, var_os}` | one environment variable, as text or raw |
| `time::init(name)` | take the zone from configuration, once, at startup (`None` = whatever the machine says) |
| `time::now_stamp()` | now, in the process's zone, as `YYYY-MM-DD HH:MM:SS` |
| `time::age_seconds(stamp)` | how old a stamp is |
| `time::days_from_stamp(stamp)` | a stamp's day count since 1970-01-01 (for weekdays) |
| `stamp::{Parts, parse, weekday, render, inject}` | the injection templates: render a template against two instants, or both halves of one message at once |
| `log::{Level, Record, push, info, warn, error, recent, by_scope, len}` | the ring |

Everything else in this folder is private.

## Relationships

- **Who asks platform**: config (the directory roots, the environment, the family, for its
  bootstrap), the api (the clock when it opens a session or lands a message, the log, the data
  directory the database file sits in), the agent (the clock when a block lands, the log, and the
  temp directory the artifact files are materialized under). Storage asks it **nothing**: every
  instant arrives as a value, and the caller reads the clock.
- **Who platform asks**: nobody of ours — the standard library and the operating system only. The
  guard table in `src/lib.rs` keeps it that way (the entry forbids this folder from naming any other
  domain).
- **One mouth for the environment**: `env.rs` is the only file that reads environment variables, and
  the only place a directory is decided. `time.rs` is the only file that reads zoneinfo. `log.rs`
  holds the process's one ring.
- **One process, one zone**: the zone is taken once and never changes; the ring and the zone are the
  two things this layer keeps in memory.

## Parts that must stay in step

1. The stamp shape `YYYY-MM-DD HH:MM:SS` — local time, no offset suffix. One producer
   (`time::now_stamp`), one parser (`stamp::parse`), and it is the shape storage stores.
2. The two template syntaxes in `stamp::render` (the letter set and the strftime set) and the
   arithmetic in `time` — the same instants, spelled two ways.
3. `os::current()` and the per-family rules `env.rs` applies: the directory names on unix, and the
   fallback everywhere else.
4. The log's record shape (level, scope, message, milliseconds) — whoever reads the ring, including
   a front end, sees exactly these fields.
5. `time::init` and the lazy fallback in `time.rs`: if nobody initializes the zone, the machine's own
   wins. A process that wants the configured one has to call `init` before its first stamp.

## Not wired yet

1. **The log ring has no reader outside the process.** `recent` and `by_scope` have no caller, so
   what this layer narrates is only visible while a debugger is attached; a front end would need a
   door of its own (`src/ipc/README.md`).
2. **`stamp::inject` has no caller.** The timestamp injection happens when a user message is written,
   and where exactly that is stays undecided until the instant-messaging mode gets its own design
   (`memo.md`); `config.server.platform.stamp.*` has no reader for the same reason.
3. **Windows.** `env.rs` answers every directory with the current directory on a non-unix family, and
   the rest of the program is unix-shaped: zoneinfo paths, `setsid`/`kill` in the bash tool, the
   transport worker's `kill -KILL`, and the browser paths. 第 16 条 lists the sites.
