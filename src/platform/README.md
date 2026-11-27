# platform

The only place in the program that asks the operating system anything; everyone else asks this layer, and this layer asks nobody.

## 1. The design

One file per question, and no question handled twice: what this system is (`os.rs`), what time it is (`time.rs`), how a stamp renders into a message (`stamp.rs`), where the process's directories are (`env.rs`), which shell runs a command line (`shell.rs`), how one command's process tree is started and stopped (`proc.rs`), what a session means by "stop" (`stop.rs`), and what happened to us (`log.rs`).

### The shell

```text
Shell { kind: Bash | Zsh | Pwsh | Niu, program }

Shell::detect()                      probe order: `$SHELL` when it names bash or zsh, then bash, zsh on unix; pwsh, niu on windows
Shell::resolve(name)                 a configured name → the backend; only a concrete name; a foreign or missing shell is refused
Shell::command(line)                 the program, its one-shot shape, and the UTF-8 setup the backend needs
Shell::path_token(path)              the one quoted token naming a stored text as a file
Shell::guarded()                     unix shells only
```

The backend is a value, not a trait: the tool receives a resolved `Shell` and never asks which system it is on, `names()` lists what this build runs, and configuration names one of them.

### The process tree

```text
Process::spawn(origin, cmd)          start a tree that follows the main process
Process::spawn_detached(origin, cmd) start a tree that outlives the front end (front end only)
process.take_stdin/stdout/stderr()   take one stream; each stream can be taken once
process.stop()                       stop the tree; the platform owns the ladder
process.try_wait/wait/wait_timeout() look at or wait for the leader's exit
```

A tree is one kernel container: its leader, every process the leader starts, and the handle that ends them together.
On unix the leader becomes its own session (`setsid`) before `exec`, and the container is the process group, signalled through the `kill` syscall.
Creation runs on `process-fork`, a thread that never ends, so `PR_SET_PDEATHSIG` binds a following tree to the main process rather than to whichever thread asked for it.
On windows the container is a job object: a following tree carries `KILL_ON_JOB_CLOSE`, a detached one does not, and `TerminateJobObject` is the single stop step.
A detached tree has no parent-death binding on either family, so the main process ending does not take it down, and it ends through its own protocol or an explicit stop.
Dropping a handle never ends a tree, because a `process-reap-<pid>` thread holds the child until it exits, reclaims leftovers, publishes the status and adds one exit record, and birth adds one record too.
Nothing here spawns `setsid`, `kill`, `taskkill` or `timeout` as a program.

### The stamp

One shape, everywhere: `YYYY-MM-DD HH:MM:SS`, local time, no offset suffix, and a stamp is an opaque string to whoever stores it.

### The templates

A template carries two names, `{{now}}` and `{{last}}`, with an optional format after a colon (`{{last:yyyy-mm-dd}}`).
A format string may mix the letter names (`yyyy`, `yy`, `mm`, `dd`, `hh`, `mi`, `ss`, `day`) with the `%`-prefixed ones (`%Y`, `%y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%F`, `%T`, `%%`), with the letter names matched longest first.
Anything unrecognized is copied through, so a typo stays visible, and a moment a template asks for and does not get means that half, prefix or suffix, is not injected at all.

### The offset

One fixed offset, written as `UTC+8`, `GMT-5` or `UTC+8:30`, which the process keeps for its lifetime.
A missing or geographic name falls back to the machine's own zone, read once at startup and reduced to its offset, and on Android the runtime is asked for the offset in effect, so a `TZ` the user set counts.

### The directories

On unix they follow XDG, where the environment variable wins and the specification's default applies when it is unset.
On windows configuration lives under `%APPDATA%` and data under `%LOCALAPPDATA%`, so a roaming profile never carries the database or the browser profile, and empty counts as unset.
One thing here is not the platform's own answer: what `~` stands for.
It is one directory for the whole process, configured through `config.server.platform.home` or, when nothing is configured, the platform's home above.
`env::expand` is the only reader of a leading `~`, and the front end asks the same question through the api instead of reading the environment itself.

### What is asked of the system, and when

System identity is decided at compile time, because a binary is built for one target and nothing probes at run time, and the environment is read on demand through this layer's one mouth.

### Invariants

1. **One mouth for the environment.** One file reads environment variables, and every other caller asks it by name.
2. **One offset per process.** Taken once at startup, from the configuration, else the machine.
3. **Values come in, never read out.** This layer opens no configuration file and names no configuration type, because the layer above reads the file and hands plain values over, and since configuration asks this layer for directories, this layer must never reach back into it.
4. **The log never lands.** A bounded ring in memory, which a restart loses.
5. **A moment is taken where it happens.** Injection reads the clock before the message is written, because there is no row to read it back from yet, and a block's own time is taken when that block lands, by whoever lands it.
6. **A shell is a value.** No caller asks this layer which system it is on, because the resolved `Shell` carries every difference a command line has.
7. **One stop, one ladder.** Every caller stops through `Process::stop`, and the platform picks the unix ladder (TERM, a fixed grace, KILL) or the windows single step.
8. **A handle is not the tree.** Dropping a `Process` ends nothing, and the tree lives until its leader exits, another entry stops it, or the main process ends.

### What lives in memory

Two things, and only these: the process's offset, and the log ring; a shell is resolved on demand and handed to its caller, so this layer keeps no default shell of its own.

## 2. Design intent

1. **One mouth for the environment.** Environment variables are read in one place, so a directory is decided once.
2. **One offset per process, taken once.** Two stamps written minutes apart cannot disagree about the offset, and the store treats a stamp as opaque text.
3. **A bad offset name never fails the run.** The clock keeps ticking and only the label is wrong.
4. **One fixed offset, everywhere.** Every moment is read with the offset chosen at startup, so a long process keeps one label across a DST switch, and that switch is the user's to write, not the machine's.
5. **`TZ` is not consulted.** The offset comes from the configuration, else from the machine once.
6. **A typo in a format stays visible.** Unrecognized text is copied through instead of being dropped.
7. **A missing moment injects nothing.** A template that asks for a moment that is not available injects nothing, for that half.

- **Only user messages get a prefix or a suffix.** A model that sees its own messages end in a timestamp learns to emit timestamps of its own.
- **Identity is decided at compile time.** A binary knows its target.
- **The log is a ring, not a file.** It narrates our behaviour, not the conversation, so it is read while the process runs and gone when it stops.
- **Directories follow the platform's own convention.** XDG on unix and `%APPDATA%`/`%LOCALAPPDATA%` on windows, while an unknown system gets the current directory.
- **No unit tests here.** Timing, the offset and the environment are checked by running the program.

## 3. Open questions

1. **Windows.** The shell, the process tree, the clock and the directories carry both families, but the unix parent-death signal reaches the leader only, and the rest of a following tree dies with the process group when the leader goes, which covers the usual case but not every stray child.
2. **Instant messaging has no caller.** The switch, the templates and the rendering exist, `stamp::inject` is public with no production call site, and the feature's shape is still open, so the switch may end up named something other than "instant message".
3. **A block's own time is taken where the block is made**, not here: the request half and the round call the clock and hand the stamp over with the block, and nothing in this layer reads it back.
4. **The templates are not read at the moment they are used.** Configuration hands the template text and the switch in, and who reads them, and when, is that layer's business.
5. **Two callers read the environment themselves.** The workspace and the shell guard call the standard library's temp directory and `HOME` directly instead of asking this layer, so "one mouth for the environment" is a rule the code does not keep everywhere yet.

## 4. Config

Read through `Config`; this layer opens no file.

1. `config.server.platform.timezone` — the fixed offset this process runs in (`UTC+8`), where unset or a geographic name means the machine's offset, read once at startup.
2. `config.server.platform.stamp.enabled` — whether a user message gets a timestamp prefix and suffix at all, off by default.
3. `config.server.platform.stamp.prefix` and `.suffix` — the two templates, rendered when the message is written.
4. `config.server.platform.ipc` — which cross-process endpoint kind to use, `os`, `unix` on unix or `pipe` on windows, read by the entry point rather than by this layer.

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.