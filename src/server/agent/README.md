# agent

One round of a conversation: ask the model, run what it asked for, settle the round. Everything
that is not a front end, not the database and not a config file lives here.

## 1. The design

### What one round is

A round starts when a surface submits a user message (the api lands it first, together with the
round's row) and ends when the model stops asking for tools, the user interrupts, the connection
breaks, or the tool-round fence is hit. One round runs on one thread.

Each lap is the same shape: assemble the whole history into one request, send it, decode the answer,
cut it into blocks and hand them to storage; if the answer asked for tools, run them and hand their
results over the same way. Nothing is held between laps except the state that outlives the round.

A round, step by step:

1. Build the environment the tools will see, from the session's own state.
2. Loop: the stop flag is up → stop; the fence is hit → the lap carries no tool manuals; otherwise
   run one lap (assemble, send, decode, run the tools, hand the results over).
3. The first lap attaches the request archive to the round.
4. Stop when the reply asked for no tools, when the fence lap came back, or when the request failed.
5. Put the working directory and the todo list back into the session's state, and hand the round's
   end — why it stopped, and its usage — to storage.

### The three parts

| part | what it is |
|---|---|
| the loop | one round: asks, runs tools, settles |
| the request half | everything about one outgoing request: assembly out, decode in, the wire between |
| the tools | the table, the front door, and one folder per tool |

### The loop's outside shape

| item | what it is |
|---|---|
| `Live` | the state that outlives a round, one per session: the provider connection, the working directory **in memory**, the tool roster, the todo list, the session's stop flag, the artifact handle, and the settings in effect. The api holds it and lends it out |
| `Settings` | the config values a round runs with |
| `Turn` | `session` + `turn_id`: the round this is |
| `Stop` | why the loop stopped: the model, the user, a broken connection, the fence |
| `Ended` | the stop reason plus the round's summed usage |
| `Outlet` | where the loop speaks while it runs: content increments, reasoning increments, a tool's running output, a tool call's three moments |
| `run` | one round, from start to settle |

**The loop does not own a session.** Every call builds its working state from the session's row, the
config and its transcript, and gives it back when the round is over. Two rounds of one session never
run at once — the api's set of running rounds is what enforces that, not this layer.

### The request half

Three entry points, and the machinery behind them:

| item | what it does |
|---|---|
| `start` | one request: rebuild the history, assemble, send, decode (streaming, handing increments to a sink), cut the reply into blocks, archive the request |
| `hand_results` | cut the tool results of one lap into blocks; one result per call, paired by `call_id` |
| `rebuild` | the history as messages: from the newest compaction on, every block of every round in it, in order. The `context` tool's window is this same list |

Behind them sit three things: **assembly** (stored messages → protocol messages, the prepared request
a summary uses), **protocols** (the request body, per-host compatibility, a streamed response → a
reply, the scanner for hosts that carry tool calls inside text, the markup filter), and **transport**
(hand a body out and read lines back, one connection per provider, and the subprocess a long
connection is killed through).

### The system prompt

It goes in at exactly one place, as the **first** protocol message of a request. Rebuilding the
history never injects anything, so the bytes a request carries never drift between laps or restarts.
A compaction replays the live context, and its system prompt goes in the same way.

An empty system prompt is not written: "no prompt" and "an empty prompt" are different things, and
whoever reads the archive has to be able to tell them apart.

### Blocks: how a reply is cut

| what the model produced | what lands |
|---|---|
| a thinking chain | one `reasoning` block, before the rest |
| tool calls | one `tool_request` block per call, paired by `call_id`; the reply's own text and the model's one-line intent ride the **first** call (a flag marks it), so replaying groups them back into one protocol message |
| plain text | one `assistant` block; an empty reply lands the built-in placeholder text, and an interrupted one its own |
| a tool's result | one `tool_result` block per call; `ok` colours the card, `memo` is the front end's structured payload, `result` is what the model reads |
| a `kairos` wake | one `wake` block; it reads back as a user message wrapped in `[system wake: …]` |

The cut lives in the request half only, and the columns live in storage; nothing else decides what a
block is.

### The tools

| item | what it is |
|---|---|
| `Tool` | one row of the table: name, manual, parameter list, and one `run` function |
| `TOOLS` / `roster` | the table, and the table filtered by the profile's gate, as the model sees it |
| `execute` | the front door: look the name up, check the gate, fill the arguments, run it, then measure the result |
| `ToolOutput` | `text` (for the model), `memo` (for a front end), `spill` (the whole text, when the result is too big) |
| `kairos` | the main-session background task tool: accept work now/later/on a cron, wake on completion, fetch output with `result` |
| `task` | the main-session tool that spawns one subagent from a template |
| `final` | the child-only tool that edits the draft and writes the final file |
| `mcp` | the static MCP tool: list configured servers/tools or call one tool through the bridge |
| `jobs` | the per-session async framework: schedules, pool, wake, output retention |
| `subagent` | the child service: template, contract, spawn, agent runner, draft |
| `Env` | everything a tool may touch: the workspace, the output budget, the todo list, the history window, the artifact handle, the bash timeout and stop flag, the browser package, the profile gate, the jobs handle, the subagent handle, and the MCP bridge |
| `Workspace` | the working zone, and how a path is resolved inside it |

A tool reads nothing itself: no database, no config file, no global. Whatever it may use is in `Env`,
put there by the loop. The `web` tool is compiled in only with the `web` feature; without it its row
is not in the table at all. The `mcp` tool is always in the table; it is hidden from the roster when
no `mcp.servers` are configured, and the profile or child template can still exclude it.

### MCP

`mcp` is one static tool, not a dynamic registry. `list` asks the bridge for servers and their tools;
`call` forwards one JSON object and returns text. `mcp list` data can change, but `TOOLS` keeps one
row.

`McpBridge` is the only backend seam. It returns `ServerInfo`, `ToolInfo`, `McpResult` and
`McpContent`; it never sees `ToolOutput`, rmcp or tokio. The default `SyncMcpBridge` owns one lazy
stdio process per configured server, shared by every session. A server that dies is restarted on the
next call; `shutdown` kills it.

The tool maps MCP `isError` to a failed tool result, keeps the text, and marks image/audio/structured
content as unsupported. Text is joined in order; the front door still owns spill.

Servers live at top-level `mcp:` in `config.yaml`. `command` is direct exec; `args`, `env`, `cwd`
and `timeoutSecs` have the defaults documented in the config README. There is no `mcp` feature; the
default stdio backend is always compiled. A future `mcp-rmcp` feature would only replace the bridge
implementation. The first version does not cover resources, prompts, HTTP, OAuth, or MCP Tasks.

A tool's **parameter list is its only declaration**: the manual the model reads and the arguments a
call is filled from both grow out of it, and the model's own one-line "what I am about to do" is
injected in exactly one place, when the manual is built from the table. Adding a parameter is one
edit in one file.

`memo` has a convention: always a JSON object carrying a `kind`, and only the fields a front end
**cannot compute** for itself. What the arguments and the result text already say — the old text, the
new text, a line count, an exit code — never goes in. `edit` uses it today: the exact mode records
which line a change landed on; the mass mode records the original text of the lines it replaced.

### Background jobs

`jobs` is the session's async framework, not a tool. `JobSpec` carries a `JobRunner`, a schedule
(`Now`, `After`, `Every`), a timeout, whether it needs a pool slot, and whether it produces output.
`JobRunner` is the only extension point: `kairos` supplies command and text runners; subagents supply
the agent runner. The framework never matches a task kind.

One scheduler thread per session owns the clock, the pool, skips, cancellation and wake merging.
The pool is shared by every job of that session. `now` fails when the pool is full; `after` waits for
a slot; `every` skips an overlapping run. A reserved job stays scheduled until the API activates it,
which is how a child is started only after its anchor lands.

Jobs live in memory only. Watch zero, delete, or `Live` drop cancels every job, kills command process
groups, removes the TMP output, and emits no wake. One job has one TMP output file; `result` reads and
removes it. Retained unread results are bounded; the oldest is marked expired and removed.

### Subagents

A subagent is an ordinary session, not a second loop. It has its own row, `Live`, rounds, tools and
history. It inherits the parent's model and profile at spawn; later parent changes do not reach it.
It cannot accept `Submit` or `Compact`, cannot spawn another subagent, and does not receive `kairos`.

`task` is parent-only. It resolves a template, checks that the template's tools are a subset of the
parent's roster, creates the child row, returns one `Subsession` effect, and reserves an agent job.
The loop starts that job only after the tool result and `subsessions` anchor land, so a fast child
cannot wake before its anchor exists. The receipt names the child, job and final path; the anchor is
the landed tool-result block.

A template is markdown with a leading YAML block carrying `tools`. `{{...}}` names are rendered from
string `vars`; missing, extra, empty, or unclosed names fail. `task` in a template fails. `APPENDED`
is only a normal reserved name: it must appear in the template if supplied.

`final` is child-only and system-added; it is not part of the template's subset check. It edits one
in-memory draft (`write`, `edit`, `mass_edit`, `read`) and `submit` writes the draft to
`<parent root>/.mypi/answers/<child>.md` with a system timestamp, then stops the child. `submit`
must be the only call in its message. If the child ends without `submit`, the runner writes the
draft, else the last assistant text; if neither exists the child is marked failed.

A child's budget counts non-`final` calls across laps and rounds. At exhaustion the next request's
roster contains only `final`; rejected calls still get failed tool results. `final` never counts.
The wake reports status only; the parent reads the final file by path.

### Oversized output, end to end

1. A tool returns everything it produced. It never measures itself.
2. The **front door** measures: over the line or byte budget and the rest moves into the spill, while
   the visible text keeps whole lines (a single huge line is cut on a character boundary).
3. The loop hands the result block, spill and all, to the request half.
4. Storage lands the block, writes the artifact row keyed by the block's own id, and appends the
   citation to the result text: `#<ref>`, the block id in base36. Block, artifact and citation are
   frozen together in one transaction.
5. A citation resolves back at the other end: `bash` rewrites a standalone `#<ref>` token into a read
   of the file (a token that names nothing stays literal text), and `context` takes `#<ref>` as an
   anchor and finds the call that produced it.
6. The file behind the path is written on demand by the session's state, which materializes it under
   the system temp directory. Those files live as long as that state; when the api lets it go, they
   go with it.

### Invariants

1. The loop holds no context: every lap rebuilds the request from storage.
2. The system prompt enters at assembly only, and the stored messages never contain it.
3. Cutting a reply into blocks is the request half's job, and the only one.
4. The output gate is at the front door: a tool never measures itself.
5. A citation is a block id spelled in base36, and a block's id never changes.
6. A tool touches only what is in `Env`.
7. Nothing is invented on the way out: an unset value is never sent.
8. The job framework never names a task kind; only a `JobRunner` does.
9. A child is a normal session; its only special rules are the roster and the final file.
10. `task` is parent-only, `final` is child-only, and both are hard-rejected outside their session.
11. A child's final path is derived from the parent root and the child id; it is not stored.

### What lives in memory

One `Live` per session — the provider connection, the working directory, the roster, the todo list,
the stop flag, the artifact handle, the jobs handle, the subagent handle, the settings — built on
demand and lent out by the api. The job service keeps schedules and completed output for that
session; the subagent service keeps child specs, the pending job map, and draft text. None of it is
written to a job table or resumed across processes.

## 2. Design intent

**The loop holds no context.** Every lap rebuilds the request from storage: read the round list, take
each block, map it to protocol messages. That is what makes a restarted process, a resumed session
and a live round send the *same* bytes — and identical bytes are what a prefix cache is paid for.

**The system prompt goes in at assembly only.** One place writes it, one place reads it back (the
compaction), and the stored messages never contain it.

**Cutting lives in one layer.** Turning a reply into blocks is the request side's job, because that
side is the thing that saw the reply, and the storage side only writes the columns it is handed.
One cut, one definition of what a block is.

**The output gate is at the front door.** Tools hand up whatever they produced; the executor decides
what the conversation keeps inline and what becomes an artifact. A tool author cannot forget the
rule, and the rule cannot differ between tools.

**The spill rides the block, and the block's id is the artifact's key.** Nothing has to map a
citation to a row: the citation *is* the block, spelled in base36. That also makes the citation
stable — a block's id never changes, so a rewind, a fork or a compaction leaves it alone, and the
text the model already read stays valid.

**Tools see an `Env`, not the world.** Everything a tool may touch is put in one struct by the loop:
the working zone, the budget, the upstream-provided inbox, the stop flag. That keeps tools testable
and keeps "who may read the database" a one-line answer.

**A tool declares itself once.** One row carries the name, the manual, the parameters and the run
function, and the manual and the argument parsing both grow from the parameter list — so they cannot
drift apart, and a tool's availability is written once.

**The todo list lives in the session, not in the model's head.** The model sends one operation plus
the one item it is talking about — that item is right there in the previous result; only `view` hands
the whole list back, and every other operation answers in a line. One bad entry fails the whole
batch, so a retry cannot collide with a half-applied list.

**The bash guard is a licence, not a sandbox.** Two zones are licensed: the session's working
directory and the system temp directory. A delete that escapes both, or that names a system root, is
refused outright; there is no warning tier and no allowlist escape.

**Nothing is invented on the way out.** A compatibility field carries what the user wrote in the
model record, or nothing at all: an unset value is never sent. The rules that differ per host are
compiled in and resolved into one struct; the switches a thinking request needs are a pointer into
that struct, not a second copy.

**The fence is not a budget.** A round that keeps asking for tools re-sends its whole history every
lap, so a live conversation runs out of context long before any sane number. A child's budget is
separate: it counts non-`final` calls across its whole life and only trims the roster.

**Jobs are a framework, not a tool.** Command, text and agent work differ only in their `JobRunner`;
scheduling, the pool, cancellation, wake merging and output retention are shared. That keeps
`kairos` a thin tool face and lets a child reuse the same lifecycle without a second scheduler.

**A child is a session.** Reusing the ordinary row, `Live`, loop and relay means the child gets
history, artifacts, watch, interrupt and delete for free. The link table stores only the anchor, so
there is no parent field to keep consistent and no template snapshot to invalidate.

**A round is the unit; a session's life is the api's.** The loop itself only ever runs one round;
keeping a session's `Live` alive while somebody watches, and letting it go when nobody does, belongs
to the layer that knows who is watching.

## 3. Open questions

1. **`Message` carries serde derives nothing uses.** The block columns are the storage format; the
   derived JSON form is not read anywhere.
2. **Placeholders are not the memo's design yet.** The request side writes the built-in text into an
   empty or interrupted block, and two writers hard-code the same bytes. The memo's rule — the block
   carries the placeholder's *name*, and `config.ui.placeholder.emptyReply` / `interrupted` own the
   words — is not implemented: no server-side code reads those two keys.
3. **The write lock is the api's running set.** "One writer per session" is enforced there, not by a
   writer's identity; the real mechanism is undecided (`memo.md`).
4. **Fork mode and the context cache.** A rewind in fork mode is a `todo!()` on the api side, and
   caching the flattened message list per session is deferred.
5. **The fence and the child budget are separate mechanisms.** The loop fence limits tool rounds;
   the child budget limits non-`final` calls. They share `ToolBudget` only for the final-only
   roster, and the loop fence still has no per-round cap on ordinary sessions.

## 4. Config

Nothing in this domain opens a config file. Every value arrives as a field: through `Live.settings`,
through `Env`, or as the argument of a call.

| key | what it decides | how it arrives |
|---|---|---|
| `config.server.agent.loop.maxToolCalls` | the tool-round fence (`false` turns it off) | `ToolBudget` in `Live.driver` |
| `config.server.agent.tools.outputMaxLines` | how much of one tool result stays inline, in lines | the output limits in `Env`, used by `read` and by the spill gate |
| `config.server.agent.tools.outputMaxBytes` | the same in bytes (for few-lines-huge results) | as above |
| `config.server.agent.tools.bashTimeout` | the wall-clock cap on one `bash` command, in seconds | the bash settings in `Env` |
| `config.server.agent.tools.browser.*` | which browser the web tool drives, and how | the browser settings in `Env` |
| `config.server.agent.tools.kairos.maxConcurrent` | jobs allowed at once in one session | `KairosConfig` in the job factory |
| `config.server.agent.tools.kairos.timeoutSecs` | the default command timeout | `KairosConfig` in the job factory |
| `config.server.agent.tools.kairos.maxOutputBytes` | bytes captured for one job | `KairosConfig` in the job factory |
| `config.server.agent.tools.kairos.maxRetained` | completed unread results kept per session | `KairosConfig` in the job factory |
| `config.server.agent.subagent.maxToolCalls` | a child's non-`final` call budget | `SubagentConfig` in the subagent factory; `task` may override per child |
| `config.server.agent.profile` | which system prompt and tool roster a session runs | resolved by the config layer into `Live.settings.system` and `Live.roster`; the profile's own gate becomes the enabled set |
| `config.server.request.compact.retainTail` | the tail a compaction keeps when the command names no budget | the compaction config handed to the plan |
| `config.server.request.streaming` | how much of a running round a front end sees | not this domain: read at the backend-to-front-end edge (`src/ipc/stream.rs`) — `immediate` / `buffered` |

## See also

`ARCHITECTURE.md` — the files, the relationships, and what is not wired yet.
