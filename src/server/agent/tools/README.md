# tools

The tool layer: one table of tools, one front door for a call, and one folder per family. A tool
is a function over `Env`: it reads what it needs from `Env`, returns a `ToolOutput`, and never
opens the database, the config file, or the loop.

## 1. Design / Schema

### The table

`registry::TOOLS`, in order:

| # | tool | file |
|---|---|---|
| 1 | `read` | `bash/read/tool.rs` |
| 2 | `edit` | `bash/edit/tool.rs` |
| 3 | `write` | `bash/write/tool.rs` |
| 4 | `cd` | `bash/cd/tool.rs` |
| 5 | `bash` | `bash/bash/tool.rs` |
| 6 | `alias` | `bash/alias/tool.rs` |
| 7 | `kairos` | `kairos/mod.rs` |
| 8 | `task` | `subagent/task.rs` |
| 9 | `final` | `subagent/final.rs` |
| 10 | `mcp` | `mcp/mod.rs` |
| 11 | `web` | `web/mod.rs` |
| 12 | `memo` | `memo/memo/tool.rs` |
| 13 | `todo` | `memo/todo/tool.rs` |
| 14 | `context` | `bash/context/tool.rs` |

**The session's list is the only gate.** `roster(names)` expands a list into manuals, keeping the
rows whose names appear in it and nothing else. `engine` reads the same list through
`env.allows(name)`, so a tool absent from the list cannot be called by naming it. No tool has a
second condition of its own: whether a session gets `mcp` depends on the profile naming it, not on
a server being configured locally.

Two names are settled when a child is spawned, in `loop/turn/cycle.rs`:

- `task` is dropped from every child's list. A child never spawns a child.
- `final` is appended to every child's list. The parent does not name it and does not know it
  exists.

`web` is absent from a build without the `web` feature: the row is gone from the table itself, so
no list can reach it.

### One call

```text
a batch with the turn's stop signal
  → stop already set: the call never runs, and the batch ends there
  → registry::spec(name)
  → env.allows(name)                       the session's list; the only gate
  → budget gate (every call but `final`)
  → Args::fill(tool.params, args)          `input` → positional values
  → (tool.run)(env, &args, progress)       Start, Progress and End events around it
  → a stop that landed meanwhile makes the result `ok: false`, reason `interrupted`
  → every out.effects gets one ts
  → spill: over the inline budget, the full text moves to out.spill
  → ToolResult
```

Unknown names error.

### Schema

A tool declares `params: &[Param]`; `Param { name, kind, default, about }` and
`Kind = Text | Number | Line | Flag | Object | Choice(&[str]) | TextList | Numbers`.

Every call is one envelope: `intent` beside `input`, both required, everything else inside `input`.
`schema_of` builds it once for every tool, so no tool declares `intent`.

- `about` says what the tool does and what it answers. No field list.
- `input`'s description says, per `type`, which fields a call needs and what an omission means.
- Each field says what its value is; its type and enum come from the table.
- No `required` array inside `input`: which fields are needed depends on `type`, and only the tool
  knows that. `Args::fill` unwraps the envelope and fills defaults; a tool that needs a field errors
  where it reads it, naming the field.
- The front door refuses a call whose `intent` is missing or blank; a field placed outside `input`
  is refused by name.

Every field is optional in the schema; `always` below marks the ones every call of that tool needs
(the tool checks them, not the schema).

| tool | parameters, in index order |
|---|---|
| `read` | `path` always, `offset` (1-based, default 1), `limit` (default 200, at most 500, capped by the output line limit) |
| `edit` | `type` always (`exact`/`mass`), `path` always, `old`, `new`, `range` (`12-14` or `12`) |
| `write` | `path` always, `content` always |
| `cd` | `path` always |
| `bash` | `command` always |
| `alias` | `type` (`add`/`list`/`del`), `name`, `command` |
| `kairos` | `type` (`now`/`after`/`every`/`list`/`status`/`result`/`cancel`), `kind` (`command`/`text`, default `command`), `content`, `timeout`, `delay`, `expr`, `count` (default 3), `id` |
| `task` | `template` always, `vars` (object of strings), `cwd`, `maxToolCalls`, `name` |
| `final` | `type` (`write`/`edit`/`read`/`submit`), `content`, `old`, `new`, `range` |
| `mcp` | `type` (`list`/`call`), `server`, `tool`, `args` (object) |
| `web` | `type` (`read`/`search`/`open`/`click`/`fill`/`select`/`scroll`/`eval`/`screenshot`), `url`, `raw` (default false), `query`, `provider`, `pages` (default 1), `selector`, `value`, `pixels` (default 600), `full_page` (default false) |
| `memo` | `type` (`add`/`mod`/`del`/`show`/`history`/`list`), `scope` (`current`/`global`, default `current`), `name`, `content` |
| `todo` | `type` (`set`/`tick`/`view`), `items` (the whole list), `ids` (item numbers, 1-based), `done` (default true) |
| `context` | `type` (`query`/`show`), `anchor` (a `#<ref>` hash or a block id), `direction` (`before`/`after`, default `after`), `limit` (default 3), `tool` (a name or `all`, default `bash`), `ids` |

`provider` is read by `search` only: one search runs against exactly one provider, so a call that
names none is refused rather than spread across several tabs, and the other operations leave it out.

`cd` lands its result with `memo {kind: "cwd", path, from}`: the history card relativizes from it
and the front end marks the directory as a temporary move.

### ToolOutput

```text
ToolOutput { text, fail: Option<String>, memo: Option<Value>, effects: Vec<Effect>, stop: bool }
ToolResult { call_id, ok, result, reason: Option<String>, memo, spill, duration_ms, effects, stop }
Spill      { name: String, content: String }
```

- `text` is what the model reads; `memo` is the structured payload the front end renders.
- `fail` is why a call was cut short; any value makes the result `ok: false`. The front door moves
  it to `reason` and appends `[reason]` after the body and its artifact note, so the marker is the
  result's last line whatever the truncation did.
- `spill` is filled by the front door, never by a tool.
- `effects` are side effects that must land with this tool result. A tool appends one; the front
  door stamps its `ts` before returning.
- `stop` asks the loop to end the round.

### Contract

`contract/` groups the tool layer's outward shape by what a caller reaches for. No name is
re-exported at the `tools` root; callers write the group they mean.

| module | holds |
|---|---|
| `contract::call` | `Batch`, `Cwd` |
| `contract::output` | `ToolResult`, `ToolOutput`, `Effect`, `MemoOp`, `AliasOp` |
| `contract::env` | `Env`, `Defaults`, `Limits`, `BashEnv`, `Paths` |
| `contract::view` | `Source`, `Export`, `Child`, `Draft`, `MemoRead`, `MemoScope`, `MemoReadOp`, `MemoVersion`, `MemoView`, and the history shapes `CallRecord`, `BlockPair`, `Direction` |
| `contract::job` | `JobSpec`, `Schedule`, `JobHost`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, `Exit`, `OutputSink` |

### Env

```text
Env {
  workspace: Workspace,      // the session's shared cwd handle
  limits: Limits,            // lines, bytes
  bash: BashEnv,             // timeout, shell
  stop: Arc<Stop>,           // the session's stop signal; see Stopping
  session: i64,
  batch_len: usize,
  tools: BTreeSet<String>,   // the session's list; the one gate
  aliases: Aliases,          // config layer above the session layer
  default_budget: u32,
  export: Arc<dyn Export>,   // a full text out of the database as a file
  paths: Paths,              // agents, answers, globalAgents
  source: Arc<dyn Source>,   // reads the loop owns
}
```

Ports are traits in `contract::view`: `Source` (memos, aliases, history, calls, jobs, child, and
the MCP bridge), `Export` and `Child`. A tool that needs a port fails when it is `None`; it never
reaches for a database or a config file.

`Env` carries no browser settings. The browser decides how it starts; see `src/web/README.md`.

### Workspace

```text
Workspace { shared }
```

- `cwd` reads the handle as it stands; `chdir` writes it. The api's `/cdp` and the model's `cd`
  write the same handle, so a move lands for both at once.
- `resolve` licenses two zones only: the live `cwd` and the temp directory. A path outside both is
  refused; `must_exist` adds the file checks.
- `cd` is licensed by one rule, `may_stand_in`: not the filesystem root, not a top-level directory
  (`/tmp` and its children excepted), and nowhere under the home tree. Everything deeper is fair
  ground, and the licensed zones then follow `cwd`.
- Where a session may stand is asked in two places only: `init` and `bind` land a barred or
  missing launch directory in the temp dir (`standing_dir`: it must exist, be a directory, and pass
  `may_stand_in`; a row whose directory has since gone lands there too), and `cd` refuses it
  outright. `/cdp` asks only whether the directory exists.
- `with_write(path, f)` takes a process-wide write lock keyed by that path; `write_atomic` writes a
  sibling temp file and renames it over the target. `read` takes no lock.
- `bash` takes no lock.

### Memo and todo

- `memo` writes (`add`, `mod`, `del`) are `current` only; a `global` write is refused. Reads
  (`show`, `history`, `list`) take both scopes.
- A memo write returns placeholder text and one `Memo` effect; the loop lands it in the same
  commit as the result block, so the tool itself never stores a note.
- `memo` reads go through `Source::memo`; the tool never opens the database.
- Three operations: `set` replaces the list (`items: []` clears it), `tick` marks item numbers done
  or, with `done: false`, clears the tick, and `view` prints the list with those numbers. Numbers
  are positions; an out-of-range number drops the whole call and nothing changes.
- A todo result's memo carries the whole list, so a front end maintains it by replacement. A
  fully-ticked list stays until the next `set`; nothing clears itself.

### Bash order

```text
admit:  expand_alias  →  rewrite #<ref>  →  guard::classify     then  shell::run
```

`admit` is the only way raw text becomes a spawnable line; `bash` and `kairos` both call it. `guard`
sees the final command. It returns `Allow`, `Warn(ids)` or `Block(ids)`; a blocked command never
starts. `kairos` admits at tool-call time and freezes the admitted line, so a job exists only for a
command that already passed. `alias add` runs the same classifier on the command it stores.

One call may carry several lines; the shell runs them in order. A bare newline is a separator to the
guard, a backslash before a newline is a continuation, and `expand_alias` rewrites the head word of
every segment once.

The command itself runs in the session's shell (`Env.bash.shell`): `bash` or `zsh` on unix, `pwsh`
or `niu` on windows. `platform::shell` builds the command line and the quoted path a `#<ref>` becomes;
`platform::proc` starts it as its own process group and stops it group-wide, TERM then KILL. A stop
arrives through the `Stop` the runner registered after the spawn; the tool thread itself only
receives, so nothing wakes it on a timer. Stdin is the null device: a command that prompts reads
EOF, never the terminal the front end owns. The guard belongs to the unix shells only; `pwsh` and
`niu` run the line as written.

### context

- `query` needs `anchor` and `direction`; `tool` is one name or `all`. `anchor` takes a `#<ref>`
  (the hash printed when a result was truncated) or a block id. The answer is a JSON array of
  `{id, tool, ok, summary}`, one element per recorded call.
- `show` answers a JSON array of `{id, request, result}`; request and result are the stored text
  parsed back into JSON, `null` when the block carries none.
- Past ten rows, or past three shown blocks, the text lands in `Export::dir()` (`<temp>/mypi`) and
  the reply names the file. The name carries the session and the parameters, so the same query in
  one session overwrites its own file and no two sessions collide. The memo still carries every row.

### Query answers and summaries

The tools that read history answer JSON, and the same value rides along as the memo: `context`
`query`/`show`, `memo` `show`/`history`/`list`, `alias` `list`, `kairos`
`now`/`after`/`every`/`list`/`status`, `mcp` `list`. The front end draws what the memo carries and
does not parse the text.

Every tool declares `summary`, one line that names the call in a history listing; the field is not
optional, so no tool joins the table without one. It carries only the least that locates the call:
the path for `read`, `write`, `edit` and `cd`, the truncated command for `bash`, the name and
command for `alias add`, the operation for the tools whose parameters are the interesting part
(`memo`, `todo`, `context`, `kairos`, `mcp`, `final`), the template and its answer name for `task`,
and for `web` the operation plus the argument it names: URL, query, selector or expression.
A tool name the table no longer holds shows its clipped arguments.

### edit

`type: exact` needs `old` (one unique run) and `new`; `type: mass` needs `range` (`12-14` or `12`)
and `new`, which may be empty to remove the range. The other group's fields are refused.

The answer names the lines that went, the lines that took their place, and how far the rest moved:

```text
replaced lines 12-14 (3) with 5 lines; below +2; file now 144 lines: /p/src/foo.rs
```

### edit's memo

```text
exact: { "kind": "diff", "atLine": 12 }
mass:  { "kind": "diff", "old": "<the replaced lines, no trailing newline>" }
```

A memo holds only what the request cannot supply: `exact` already carries `old` and `new`, so it
names the line the match sat on; `mass` already carries the range, so it carries the text that was
there. New line numbers, counts and the shift are the front end's arithmetic.

### Stopping

`Stop` lives in `platform::stop`: a flag plus a table of actions. `set` sets the flag and runs every
action once, on the caller's thread; `on_stop` registers an action and runs it at once if the
signal already fired; the returned `Hook` takes it off again when dropped. `reset` clears the flag
at the start of a turn.

A tool that blocks registers what stopping it means: `bash` TERMs its process group and KILLs it
after `platform::proc`'s grace period. A tool that finishes in microseconds registers nothing, and
the front door does not pretend otherwise.

What a stop leaves behind depends on whether the call started:

| call | history |
|---|---|
| started | one failed result, `[interrupted: the user stopped the turn]` as its last line |
| never started | nothing: no request block, no result |

A reply cut short mid-stream keeps no calls at all. Calls that arrived whole but never ran have
their request blocks dropped before the turn's one commit. A call's own deadline is not a stop:
`bash` reports `timed out after Ns` through `ToolOutput::failure`.

`final submit` is not a stop either: its result carries `stop`, and the turn settles as `finished`.

### MCP

`mcp` is one static tool with a fixed schema. Which servers exist and which tools each exposes is
decided at runtime, so a new server needs no Rust change — only a line in `mcp.json`.

The bridge belongs to a session: the tool asks `Source::mcp(session, cwd)`, which builds one on the
first call and follows the session's cwd from then on (a move rescans the table and drops the old
pool). A session that never calls `mcp` never starts a server.

`mcp/setup.rs` reads the table for where the session stands: `<cwd>/.mypi/mcp.json` when it exists,
else `<global root>/mcp.json`.

**The project's file wins whole.** If it exists the global one is not read at all; the two are
never merged. Neither existing means an empty table, and an empty table still mounts the bridge.
The table is read where the session stood at its first `mcp` call; the next call after a `cd` reads
it again, and a changed table or directory drops the process pool. An unchanged one is a no-op.

```json
{
  "servers": {
    "local": {
      "command": "uv",
      "args": ["run", "python", "service.py"],
      "env": {},
      "timeout_secs": 120
    }
  }
}
```

`McpBridge` is the seam: `servers()`, `tools(server)`, `call(server, tool, args)`, `rescan(cwd)`,
`shutdown()`. Nothing above it knows JSON-RPC. `SyncMcpBridge` is the built-in implementation: one
child process per name, started lazily and kept until a rescan; a server spawns in the session's
working directory, so a `cwd` key in the file is not used. stdout is one JSON-RPC object per line,
a reader thread dispatches by id, stderr is drained into a 20-line tail that only surfaces in
errors; every call carries a deadline; a dead process fails every waiting call and is started again
on the next one. It uses threads and channels only — no async runtime enters this tree.

An answer is text content, concatenated in order. `isError` makes the call failed. Images, audio
and structured content are reported as unsupported. MCP's own schema is not passed to the model and
not checked here; a bad argument is the server's error to report.

### Jobs and children

- A job is created by the loop, never by a tool. The tool writes a `contract::job::JobSpec` and
  holds the session's `JobHost`; the scheduler, the pool, the per-run files and the wakes live in
  `loop/job`. A session's service is built the first time anything asks for it, so a session that
  never runs a job keeps no scheduler and no output folder.
- One job, one completion: a command finishes, a child submits, or the clock arrives. The receipt
  names the job; the wake reports it done under the same id.
- `kairos` is the tool face. A text job has no completion of its own, so it refuses `now`.
- A child is a job too: `task` resolves a template and picks an answer name, the loop writes the
  session row, and a loop-side runner books the child, nudges it three times when it ends without a
  submit, then drops it.
- The answer file is `<answers>/<name>.md`; the name is fixed at spawn, from `name` or the template's
  stem, with `-1`, `-2` when taken. `final submit` writes it and is the only thing that ends a child.
- A child takes no user messages. `task` is the only tool a child cannot hold; `final` is the only
  one it never has to ask for.

### Invariants

1. A tool reads only `Env`; no database, no config file, no loop.
2. A tool never measures or trims output; the front door does.
3. A tool names an effect kind; the front door stamps `call_id` and `ts`.
4. File tools read and write only inside the licensed zones; atomically, under a per-path lock.
5. The roster is the only way the model learns a tool exists.
6. `edit` has one path in, one read, one atomic write; `mass_edit` is not a tool.
7. `alias` expands the head word of every command segment, once, and adds no variables, functions or
   multi-line bodies. A name a session writes is one word the scanner accepts as a segment head; a
   name the config fixes is read-only for the session.
8. One call runs at a time inside a session; different sessions run concurrently.
9. A tool starts no job and writes no session row; it describes both and the loop acts.
10. The session's list is the whole gate; a tool adds no condition of its own.
11. A stop is one `Stop`; only a tool that blocks registers what stopping it means.
12. A call that never ran leaves no block and no result.

## 2. Design intent

- **One table, one front door.** Gates, argument decoding, effect stamping and output measurement
  happen once, so a new tool cannot forget one of them and cannot report output straight into the
  conversation.
- **Grouped contract.** The outward shape sits in five modules by caller intent, so a reader looking
  for "what a tool returns" does not scroll past the ports the loop implements.
- **The list decides.** A profile names what a session may call; the tool layer does not second-guess
  it. The one exception is settled at spawn, not at call time: a child's list is corrected once, so
  no check has to be repeated on every call.
- **Positional parameters.** The schema and the parser come from one `&[Param]`, so the JSON schema
  the model sees cannot drift from what the tool reads.
- **`intent` everywhere.** Every call carries one line of user-facing intention; the front end can
  show it without parsing arguments.
- **Ports, not dependencies.** Reads the tool layer must not own — memos, artifacts, jobs,
  children — arrive inside `Env` as traits, so the tools keep no database handle.
- **Effects land with their block.** A memo, alias or sub-session cannot exist without the tool
  result that caused it; one transaction owns both.
- **A guard, not a sandbox.** The model runs real shell commands; the classifier blocks the
  machine-destroying shapes and warns on the rest, and it always judges the expanded command.
- **Two questions, two rules.** File tools may touch the session's directory and the temp
  directory; `cd` answers a different one — where may a session stand at all — and its rule is a
  blacklist, not a zone list. The write lock is per path, so unrelated writes never queue behind
  each other.
- **Long work leaves the round.** `kairos` starts a job and returns; a finished job wakes the
  session, so a slow command does not hold a round open.
- **A child is a session.** Subagents reuse the loop, storage and stream; corrected lists keep a
  child from spawning children of its own.
- **One stdio server per name.** MCP servers start lazily and stay up for the process; the tool
  only knows names, so a new server is a line of JSON.
- **The browser is optional at build time.** `web` is a feature, not a runtime switch, because the
  browser stack is the largest dependency in the crate.

## 3. Unresolved

1. MCP carries text content only; a server-initiated request is answered `-32601`.
2. Template front matter reads `tools` only.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.bashTimeout` | one shell command's deadline; default 600 s |
| `config.server.agent.tools.aliases` | user aliases as `name=command`; every session reads them, nothing shadows them |
| `config.server.agent.tools.outputMaxLines` | inline line budget; also caps `read`'s `limit` |
| `config.server.agent.tools.outputMaxBytes` | inline byte budget |
| `config.server.agent.tools.browser.bin` | the browser executable; empty = `$MYPI_BROWSER_BIN`, else `/opt/helium/helium` |
| `config.server.agent.tools.browser.profileDir` | the persistent profile; empty = `<data dir>/browser/profile` |
| `config.server.agent.tools.browser.port` | attach to a running debugging port instead of launching |
| `config.server.agent.tools.browser.idleSecs` | seconds without activity before an owned browser is closed; default 600 |
| `config.server.agent.tools.browser.userAgent` | override the launched browser's User-Agent; unset = the browser's own |
| `config.server.agent.tools.browser.extraArgs` | extra browser argv |
| `config.web.bringToFront` | raise the window when a verification challenge appears; default true |
| `config.server.agent.tools.kairos.*` | `maxConcurrent`, `timeoutSecs`, `maxOutputBytes`, `maxRetained`; the job scheduler's numbers, read by the loop |
| `config.server.agent.subagent.maxToolCalls` | one child's tool-call budget; read by the loop |
| `config.server.agent.loop.tools.task.timeoutSecs` | how long one sub-session may run; default 3600 s |

The browser keys are read by the web domain itself, not by the tool layer.
