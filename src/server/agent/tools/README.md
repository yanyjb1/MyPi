# tools

The tool layer: one table of tools, one front door for a call, and one folder per family. A tool
is a function over `Env`: it reads what it needs from `Env`, returns a `ToolOutput`, and never
opens the database, the config file, or the loop.

## 1. Design / Schema

### The table

`registry::TOOLS`, in order:

| # | tool | file | reachable when |
|---|---|---|---|
| 1 | `read` | `bash/read/mod.rs` | always |
| 2 | `edit` | `bash/edit/mod.rs` | always |
| 3 | `write` | `bash/write/mod.rs` | always |
| 4 | `cd` | `bash/cd/mod.rs` | always |
| 5 | `bash` | `bash/bash/mod.rs` | always |
| 6 | `alias` | `bash/alias/mod.rs` | always |
| 7 | `kairos` | `kairos/mod.rs` | a parent session with a job service |
| 8 | `task` | `subagent/task.rs` | a parent session |
| 9 | `final` | `subagent/final.rs` | a child session |
| 10 | `mcp` | `mcp/mod.rs` | an MCP bridge is configured |
| 11 | `web` | `web/mod.rs` | the build has the `web` feature |
| 12 | `memo` | `memo/memo/mod.rs` | always |
| 13 | `todo` | `memo/todo/mod.rs` | always |
| 14 | `context` | `bash/context/mod.rs` | always |

`roster(env)` shows the model only the rows that pass `env.allows(name)` and
`registry::available(env, name)`; `executor::execute` repeats both gates, so a hidden tool cannot
be called by naming it.

### One call

```text
ToolCall { name, arguments }
  → registry::spec(name)
  → env.allows(name)                       profile / child-template set
  → registry::available(env, name)          session gate
  → Args::fill(tool.params, arguments)      JSON object → positional values
  → (tool.run)(env, &args, progress)
  → every out.effects gets call_id and one ts
  → spill: over the inline budget, the tail moves to out.spill
  → ToolOutput
```

Unknown names error. With the `web` feature off, the three historical names `fetch`, `browser`
and `search` say so explicitly instead of "unknown tool".

### Schema

A tool declares `params: &[Param]`; `Param { name, kind, required, default, about }` and
`Kind = Text | Number | Line | Flag | Any | Choice(&[str]) | TextList | Table(&[Param])`.
`schema_of` renders the JSON schema and injects one required string property, `intent`, into every
tool; no tool declares it.

| tool | parameters, in index order |
|---|---|
| `read` | `path` req, `offset` (1-based), `limit` (default 500, capped by the output line limit) |
| `edit` | `type` req (`exact`/`mass`), `path` req, `old`, `new`, `edits` table of `{line, text}` |
| `write` | `path` req, `content` req |
| `cd` | `path` req |
| `bash` | `command` req |
| `alias` | `op` req (`add`/`list`/`remove`), `name`, `command` |
| `kairos` | `op` req (`now`/`after`/`every`/`list`/`status`/`result`/`cancel`), `type` (`command`/`text`), `content`, `timeout`, `delay`, `expr`, `count`, `id`, `notify` |
| `task` | `template` req, `vars`, `cwd`, `maxToolCalls` |
| `final` | `op` req (`write`/`edit`/`mass_edit`/`read`/`submit`), `content`, `old`, `new`, `edits` |
| `mcp` | `op` req (`list`/`call`), `server`, `tool`, `args` |
| `web` | `op` req (`read`/`search`/`open`/`click`/`fill`/`press`/`select`/`scroll`/`eval`/`net`/`screenshot`), `url`, `raw`, `query`, `provider`, `page`, `selector`, `value`, `filter`, `max`, `full_page`, `reserve` |
| `memo` | `op` req (`add`/`mod`/`del`/`show`/`history`/`list`), `scope` (`current`/`global`, default `current`), `name`, `content` |
| `todo` | `op` req (`init`/`append`/`start`/`done`/`drop`/`block`/`unblock`/`rm`/`view`), `list` table of `{phase, items}`, `items`, `phase`, `task`, `reason` |
| `context` | `anchor` req, `before`, `after`, `include_user`, `include_results` |

### ToolOutput

```text
ToolOutput { text: String, memo: Option<Value>, spill: Option<Spill>, effects: Vec<Effect>, stop: bool }
Spill      { name: String, content: String }
```

- `text` is what the model reads; `memo` is the structured payload the front end renders.
- `spill` is filled by the front door, never by a tool.
- `effects` are side effects that must land with this tool result. A tool appends a kind; the front
  door writes `call_id` and one `ts` onto every effect before returning.
- `stop` asks the loop to end the round.

### Env

```text
Env {
  workspace: Workspace,
  limits: OutputLimits,      // max_lines, max_bytes
  inbox: Inbox,              // todo, window, artifacts, session, memo, jobs, subagent
  bash: BashEnv,             // timeout, interrupt
  browser: BrowserConfig,
  enabled: Option<BTreeSet<String>>,
  aliases: BTreeMap<String, String>,
  mcp: Option<Arc<dyn McpBridge>>,
}
```

`Inbox` carries the ports a tool may call — `Artifacts`, `MemoRead`, `JobService`, `Subagent` —
and the session's id. A tool that needs a port fails when it is `None`; it never reaches for one.

### Workspace

```text
Workspace { cwd, root }
```

- `resolve` licenses two zones only: the live `cwd` and the temp directory. A path outside both is
  refused; `must_exist` adds the file checks.
- `cd` may move to `root`, `$HOME` or the temp directory, and then the licensed zones follow `cwd`.
- `with_write(path, f)` takes a process-wide write lock keyed by that path; `write_atomic` writes a
  sibling temp file and renames it over the target. `read` takes no lock.
- `bash` takes no lock.

### Memo and todo

- `memo` writes (`add`, `mod`, `del`) are `current` only; a `global` write is refused. Reads
  (`show`, `history`, `list`) take both scopes.
- A memo write returns placeholder text and one `EffectKind::Memo`; the writer applies it in the
  same transaction as the tool result, so the tool itself never stores a note.
- `memo` reads need `Inbox.memo`; without the port the tool says reads are unavailable.
- `todo`'s `view` is read-only. Every other operation rewrites `env.inbox.todo` once, after the
  whole batch validates; a failed batch leaves the list as it was.

### Bash order

```text
expand_alias  →  rewrite #<ref> to a path  →  guard::classify  →  shell::run
```

`guard` sees the final command. It returns `Allow`, `Warn(ids)` or `Block(ids)`; a blocked command
never starts. `alias add` runs the same classifier on the command it stores.

### Jobs and children

- `jobs` is a per-session service: a scheduler thread, a concurrency pool, per-job output files
  under `$TMP/mypi-jobs/<session>`, and a wake channel that turns a finished job into a new round.
- `kairos` is the only model-facing door to it; `task` and `final` borrow the same service to hold
  a child's slot.
- A child session is a session: it gets its own `Env`, root and tool set. `task` is parent-only,
  `final` is child-only.

### Invariants

1. A tool reads only `Env`; no database, no config file, no loop.
2. A tool never measures or trims output; the front door does.
3. A tool names an effect kind; the front door stamps `call_id` and `ts`.
4. File tools read and write only inside the licensed zones; atomically, under a per-path lock.
5. The roster is the only way the model learns a tool exists.
6. `edit` has one path in, one read, one atomic write; `mass_edit` is not a tool.
7. `alias` expands once, the first token only, and adds no variables, functions or multi-line forms.
8. One call runs at a time inside a session; different sessions run concurrently.

## 2. Design intent

- **One table, one front door.** Gates, argument decoding, effect stamping and output measurement
  happen once, so a new tool cannot forget one of them and cannot report output straight into the
  conversation.
- **Positional parameters.** The schema and the parser come from one `&[Param]`, so the JSON schema
  the model sees cannot drift from what the tool reads.
- **`intent` everywhere.** Every call carries one line of user-facing intention; the front end can
  show it without parsing arguments.
- **Ports, not dependencies.** Reads the tool layer must not own — memos, artifacts, jobs,
  children — arrive inside `Env` as traits, so the tools keep no database handle.
- **Effects land with their block.** A memo, alias or subsession cannot exist without the tool
  result that caused it; one transaction owns both.
- **A guard, not a sandbox.** The model runs real shell commands; the classifier blocks the
  machine-destroying shapes and warns on the rest, and it always judges the expanded command.
- **Two zones.** File tools may touch the session's directory and the temp directory; `cd` may
  widen the first one to `root` or `$HOME`, and the write lock is per path, so unrelated writes
  never queue behind each other.
- **Long work leaves the round.** `kairos` starts a job and returns; a finished job wakes the
  session, so a slow command does not hold a round open.
- **A child is a session.** Subagents reuse the loop, storage and stream; separate roots and the
  parent/child tool split keep a child from spawning children of its own.
- **One stdio server per name.** MCP servers start lazily and stay up for the process; the tool
  only knows names.
- **The browser is optional at build time.** `web` is a feature, not a runtime switch, because the
  browser stack is the largest dependency in the crate.

## 3. Unresolved

1. Alias effects are stored, but no path reads them back: `Env::with_aliases` has no caller, so a
   new round starts with an empty alias map.
2. The `todo` tool mutates `Inbox.todo`; nothing copies it back into `Live.todo`.
3. Jobs live in memory only; a restart drops them, and the output directory is wiped when the
   factory is built.
4. `shell` documents TERM then KILL; only TERM is sent.
5. With `web` off, the unknown-tool error names `fetch`, `browser` and `search`, which are not
   table rows.
6. MCP carries text content only; a server-initiated request is answered `-32601`.
7. Template front matter reads `tools` only, and `ChildSpec.parent` is set but never read.
8. `JobService::session`, `JobService::sink`, `SubagentFactory::drafts` and
   `licensed_zones` have no callers.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.bashTimeout` | one shell command's deadline; default 600 s |
| `config.server.agent.tools.outputMaxLines` | inline line budget; also caps `read`'s `limit` |
| `config.server.agent.tools.outputMaxBytes` | inline byte budget |
| `config.server.agent.tools.browser.*` | `bin`, `port`, `profileDir`, `headless`, `userAgent`, `extraArgs`, handed to `Env.browser` |
| `config.server.agent.tools.kairos.maxConcurrent` | jobs running at once |
| `config.server.agent.tools.kairos.timeoutSecs` | one job's deadline |
| `config.server.agent.tools.kairos.maxOutputBytes` | one job's retained output |
| `config.server.agent.tools.kairos.maxRetained` | finished jobs kept for `result` |
| `config.server.agent.subagent.maxToolCalls` | one child's tool-call budget, passed to the factory |
| `mcp.servers.<name>.command`, `.args`, `.env`, `.cwd`, `.timeoutSecs` | one stdio MCP server |
