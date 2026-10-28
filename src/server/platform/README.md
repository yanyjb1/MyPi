# platform

The only place in the program that asks the operating system anything. Everyone else asks this
layer; this layer asks nobody.

## 1. The design

One file per question, and no question handled twice: what this system is (`os.rs`), what time it is
(`time.rs`), how a stamp renders into a message (`stamp.rs`), where the process's directories are
(`env.rs`), and what happened to us (`log.rs`).

### The stamp

One shape, everywhere: `YYYY-MM-DD HH:MM:SS`, local time, no offset suffix. A stamp is an opaque
string to whoever stores it.

### The templates

A template carries two names — `{{now}}` and `{{last}}` — with an optional format after a colon
(`{{last:yyyy-mm-dd}}`). A format string may mix the letter names (`yyyy`, `yy`, `mm`, `dd`, `hh`,
`mi`, `ss`, `day`) with the `%`-prefixed ones (`%Y`, `%y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%F`, `%T`,
`%%`); the letter names are matched longest first. Anything unrecognized is copied through, so a typo
stays visible. A moment a template asks for and does not get means that half — prefix or suffix — is
not injected at all.

### The zone

The zone is a transition table, not one offset: a long process across a DST switch still computes the
right local time, because each moment looks up its own stretch. The table comes from the zoneinfo
file the configuration names — a bare name is looked up under the system's zoneinfo directory, an
absolute path is used as it is — and `/etc/localtime` is the fallback. `TZ` is not consulted.

### The directories

On unix they follow XDG: the environment variable wins, and the specification's default applies when
it is unset. Empty counts as unset.

### What is asked of the system, and when

System identity is decided at compile time: a binary is built for one target, and nothing probes at
run time. The environment is read on demand, through this layer's one mouth.

### Invariants

1. **One mouth for the environment.** One file reads environment variables; every other caller asks
   it by name.
2. **One zone per process.** Taken once, at startup, from the configuration.
3. **Values come in, never read out.** This layer opens no configuration file and names no
   configuration type: the layer above reads the file and hands plain values over. Configuration asks
   this layer for directories, so this layer must never reach back into it.
4. **The log never lands.** A bounded ring in memory; a restart loses it.
5. **A moment is taken where it happens.** Injection reads the clock before the message is written,
   because there is no row to read it back from yet; a block's own time is taken when that block
   lands, by whoever lands it.
6. **Tests are not a concern of this layer.** Timing, zone tables and the environment are checked by
   running the program; this folder carries no unit tests.

### What lives in memory

Two things, and only these: the process's zone, and the log ring.

## 2. Design intent

**One mouth for the environment.** Environment variables are read in one place, so a directory is
decided once.

**One zone per process, taken once.** Two stamps written minutes apart cannot disagree about the
offset, and the store treats a stamp as opaque text.

**A bad zone name never fails the run.** The clock keeps ticking and only the label is wrong.

**A transition table, not one offset.** Each moment looks up its own stretch, so a long process
across a DST switch still computes the right local time.

**`TZ` is not consulted.** The zone is a configured value and the machine is the fallback.

**A typo in a format stays visible.** Unrecognized text is copied through instead of being dropped.

**A missing moment injects nothing.** A template that asks for a moment that is not available
injects nothing, for that half.

**Only user messages get a prefix or a suffix.** A model that sees its own messages end in a
timestamp learns to emit timestamps of its own.

**Identity is decided at compile time.** A binary knows its target.

**The log is a ring, not a file.** It narrates our behaviour, not the conversation: read while the
process runs, gone when it stops.

**Directories follow the platform's own convention.** XDG on unix; an unknown system gets the
current directory.

**No unit tests here.** Timing, zone tables and the environment are checked by running the program.

## 3. Open questions

1. **Windows.** The crate has never been compiled there and no dependency has been checked for it, so
   the promises above hold on unix only: the zone table's paths, the XDG directories, the shell tool
   (`setsid`, `kill`, `/bin/sh`), the transport's process teardown and the browser paths are all
   unix-shaped (第 16 条). `os.rs` exists so the day it matters, the question has one place to ask.
2. **Instant messaging has no caller.** The switch, the templates and the rendering exist;
   `stamp::inject` is public with no production call site, and the feature's shape is still open — the
   switch may end up named something other than "instant message".
3. **A block's own time is taken where the block is made**, not here: the request half and the round
   call the clock and hand the stamp over with the block, and nothing in this layer reads it back.
4. **The templates are not read at the moment they are used.** Configuration hands the template text
   and the switch in; who reads them, and when, is that layer's business.
5. **Three callers read the environment themselves.** The workspace, the bash guard and the `cd` tool
   call the standard library's temp directory and `HOME` directly instead of asking this layer, so
   "one mouth for the environment" is a rule the code does not keep everywhere yet.

## 4. Config

Read through `Config`; this layer opens no file.

| key | what it decides |
|---|---|
| `config.server.platform.timezone` | the zone this process runs in; unset means whatever the machine says |
| `config.server.platform.stamp.enabled` | whether a user message gets a timestamp prefix and suffix at all (off by default) |
| `config.server.platform.stamp.prefix` / `.suffix` | the two templates, rendered when the message is written |
| `config.server.platform.ipc` | which cross-process endpoint kind to use (`os` / `unix` / `pipe`); read by the entry point, not by this layer |

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
