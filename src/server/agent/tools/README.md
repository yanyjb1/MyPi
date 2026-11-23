# tools

The tool layer: one table of tools, one front door for a call, and one folder per family. A tool is a function over `Env`: it reads what it needs from `Env`, returns a `ToolOutput`, and never opens the database, the config file, or the loop.

Each tool's own manual lives beside its code. This file holds only what every tool shares: the list, how a session enables one, the call's common path, and where each manual is.

## 1. Design / Schema

### The table

`registry::TOOLS`, in order:

| # | tool | file |
|---|---|---|
| 1 | `read` | `fs/read/tool.rs` |
| 2 | `grep` | `fs/grep/tool.rs` |
| 3 | `edit` | `fs/edit/tool.rs` |
| 4 | `splice` | `fs/splice/tool.rs` |
| 5 | `write` | `fs/write/tool.rs` |
| 6 | `cd` | `cd/tool.rs` |
| 7 | `shell` | `shell/tool.rs` |
| 8 | `alias` | `shell/alias/tool.rs` |
| 9 | `kairos` | `kairos/tool.rs` |
| 10 | `task` | `subagent/task.rs` |
| 11 | `final` | `subagent/final.rs` |
| 12 | `mcp` | `mcp/mod.rs` |
| 13 | `web` | `web/mod.rs` — feature `web` |
| 14 | `search` | `search/mod.rs` — feature `web` |
| 15 | `memo` | `memo/memo/tool.rs` |
| 16 | `todo` | `memo/todo/tool.rs` |
| 17 | `context` | `memo/context/tool.rs` |

### Enabling a session

**The session's list is the only gate.** `roster(names)` expands a list into manuals, keeping the rows whose names appear in it and nothing else. `engine` reads the same list through `env.allows(name)`, so a tool absent from the list cannot be called by naming it. No tool has a second condition of its own: whether a session gets `mcp` depends on the profile naming it, not on a server being configured locally.

Two names are settled when a child is spawned, in `loop/turn/cycle.rs`:

- `task` is dropped from every child's list. A child never spawns a child.
- `task`'s `model` names one `provider:model` from the roster the loop copied out of `models.yml` when it was built; an unknown address is refused with the roster printed, and `list` prints it on demand. A child without `model` runs on its parent's model, as before. The roster is a loop-start snapshot: models.yml edits land with the next loop, like every other tool fact.
- `final` is appended to every child's list. The parent does not name it and does not know it exists.

`web` drives the session's work tab, so its operations sit in separate calls; `search` holds the session's search tab and names nothing else.

### One call

```text
a batch with the turn's stop signal
  → stop already set: the call never runs, and answers that it never ran
  → registry::spec(name)
  → env.allows(name)                       the session's list; the only gate
  → budget gate (every call but `final`)
  → Args::fill(tool.params, args)          fields → positional values
  → (tool.run)(env, &args, progress)       Start, Progress and End events around it
  → a stop that landed meanwhile makes the call failed, reason `interrupted`
  → every out.effects gets one ts
  → spill: over the inline budget, the full text moves to out.spill (`read` excepted: it bounds itself)
  → ToolResult
```

After the batch's last call, every written call's shell reaches `fs/fault.rs`'s audit: one check per unit written into, run by the language that owns it. The audit's rules — where each diagnostic lands, the one insert pass, and what the last written answer carries — are in `fs/splice/README.md`.

Unknown names error.

### Schema

A tool declares `params: &[Param]`; `Param { name, kind, default, about }` and `Kind = Text | Number | Line | Flag | Object | Choice(&[str]) | TextList | Numbers | Objects(&[Param])`.

Every call is one flat object: `intent` beside the tool's own fields, all at one level. `schema_of` adds `intent` once for every tool, so no tool declares it.

`Objects` is an array of objects, and its items are the one place a field is read by name: a position indexes a call's top level only. Its own `Param` table is rendered under the array's `items`, and `Args::objects` hands back one `Item` per element, taking defaults, kinds and enums from that same table. An item error names itself, `files[2].path`.

- `about` says what the tool does and what it answers. No field list.
- `fields` says what the fields are and which ones a call needs per `type`. No prose the schema already carries: a field's own `about` says what its value is.
- Each field says what its value is; its type and enum come from the table.
- Only `intent` is `required`: which other fields are needed depends on `type`, and only the tool knows that. `Args::fill` reads the fields by table and fills defaults; a tool that needs one errors where it reads it, naming the field.
- A name the table does not hold is refused by `Args::fill` before the tool runs, and an object array refuses one of its items' unknown fields the same way: a call that names `range` to `edit` hears back the names `edit` takes.
- The front door refuses a call whose `intent` is missing or blank.

Every field but `intent` is optional in the schema; a tool checks the fields its own call needs.

Each tool's parameters and their rules are in its manual, listed at the end of this file.

### Batched calls

Five tools take an array where they used to take one item: `read`, `alias`, `kairos`, `mcp` and `task`. `edit`, `write`, `memo`, `context`, `final` and `web` stay single, because their payloads are large enough that a batch would only move the same bytes into one argument. `splice` takes an array for a different reason: its items are ranges of one file, applied to one snapshot, so a batch is one write instead of several that would each need the file read again.

- Items run in the array's order, one after another; the answer is laid out in the same order.
- Items are isolated: one item's failure is written where that item would have answered, and the rest still run and still answer. The call fails when any item failed, and `fail` counts them.
- Every item states its own outcome in `items`, aligned with the arguments' order.
- A query operation — `alias list`, `kairos list/status/result/cancel`, `mcp list` — takes no array and refuses one.
- A batch whose items produce structured payloads answers a memo array, and the history cards draw one entry per item.
- A call with exactly one item answers as the single-item call always did: its one item's failure is the call's failure, with no item label in the text.

### ToolOutput

```text
ToolOutput { text, images: Vec<ImagePart>, items: Vec<ItemOk>, fail: Option<String>, memo: Option<Value>, effects: Vec<Effect>, stop: bool }
ToolResult { call_id, items: Vec<ItemOk>, result, images: Vec<ImagePart>, reason: Option<String>, memo, spill, duration_ms, effects, stop }
Spill      { name: String, content: String }
ItemOk     { ok: bool, error: Option<String> }
ImagePart  { format: String, data: String }  // base64; empty images is the text-only shape
```

- `text` is what the model reads; `memo` is the structured payload the front end renders.
- `items` is what each item of the call answered, in the arguments' order, and it is the only source of the front end's status: the card draws one row per item and hangs that item's `error` under it. A single-item tool leaves it empty and the front door answers one entry.
- `fail` is why a call was cut short; any value fails the call. The front door moves it to `reason` and appends `[reason]` after the body and its artifact note, so the marker is the result's last line whatever the truncation did. A cut call states its reason in its one item.
- `spill` is filled by the front door, never by a tool.
- `effects` are side effects that must land with this tool result. A tool appends one; the front door stamps its `ts` before returning.
- `stop` asks the loop to end the round.

### Query answers

The tools that read history answer JSON, and the same value rides along as the memo: `context` `query`/`show`, `memo` `show`/`history`/`list`, `alias` `list`, `kairos` `now`/`after`/`every`/`list`/`status`, and `mcp` `list`. The front end draws what the memo carries and does not parse the text.

Every tool declares `summary`, one line that names the call in a history listing; the field is not optional, so no tool joins the table without one. It carries only the least that locates the call.

### Contract

`contract/` groups the tool layer's outward shape by what a caller reaches for. No name is re-exported at the `tools` root; callers write the group they mean.

| module | holds |
|---|---|
| `contract::call` | `Batch`, `Cwd` |
| `contract::output` | `ToolResult`, `ToolOutput`, `Effect`, `MemoOp`, `AliasOp` |
| `contract::env` | `Env`, `Defaults`, `Limits`, `ShellEnv`, `Paths`, `Seen`, `Stamp`, `Answer` |
| `contract::view` | `Source`, `Export`, `Child`, `Draft`, `MemoRead`, `MemoScope`, `MemoReadOp`, `MemoVersion`, `MemoView`, and the history shapes `CallRecord`, `BlockPair`, `Direction` |
| `contract::job` | `JobSpec`, `Schedule`, `JobHost`, `JobRunner`, `JobCtx`, `JobOutcome`, `Receipt`, `Row`, `Status`, `Exit`, `OutputSink` |

### Env

```text
Env {
  workspace: Workspace,      // the session's shared cwd handle
  limits: Limits,            // lines, bytes
  shell: ShellEnv,           // timeout, backend
  stop: Arc<Stop>,           // the session's stop signal; see Stopping
  session: i64,
  batch_len: usize,
  tools: BTreeSet<String>,   // the session's list; the one gate
  aliases: Aliases,          // config layer above the session layer
  default_budget: u32,
  export: Arc<dyn Export>,   // a full text out of the database as a file
  vision: bool,              // the session's model takes image parts; read's gate
  persistent_docs: Arc<[String]>, // document stems the user keeps current; a listing never weighs them
  source: Arc<dyn Source>,   // reads the loop owns
  seen: Seen,                // the module cards shown, and every file's read stamp
  batch: Vec<ToolCall>,      // the calls now running, so one can merge with another in the same batch
  current: String,           // the call now running
  handed: BTreeMap<String, Answer>, // what a merged run already answered for a later call in the batch
}
```

`batch`, `current` and `handed` are per batch, not per session: the front door fills them before the first call and clears the ledger with it. A tool that merges work (only `splice` does) answers for the other calls once and hands each its own result when its turn comes.

Ports are traits in `contract::view`: `Source` (memos, aliases, history, calls, jobs, child, and the MCP bridge), `Export` and `Child`. A tool that needs a port fails when it is `None`; it never reaches for a database or a config file.

`Env` carries no browser settings. The browser decides how it starts; see `src/web/README.md`.

### Workspace

```text
Workspace { shared }
```

- `cwd` reads the handle as it stands; `chdir` writes it. The api's `/cdp` and the model's `cd` write the same handle, so a move lands for both at once.
- `resolve` licenses two zones only: the live `cwd` and the temp directory. A path outside both is refused; `must_exist` adds the file checks.
- `with_write(path, f)` takes a process-wide write lock keyed by that path; `write_atomic` writes a temp file beside the target and renames it into place. `read` takes no lock.
- `shell` takes no lock.

Where a session may stand, and the rule `cd` answers, is in `cd/README.md`.

### Stopping

`Stop` lives in `platform::stop`: a flag plus a table of actions. `set` sets the flag and runs every action once, on a thread of its own; `on_stop` registers an action and runs it at once if the signal already fired; the returned `Hook` takes it off again when dropped. `reset` clears the flag at the start of a turn. `pause` sleeps until the flag or its deadline, whichever comes first, and answers whether the pause ran out: a wait that would otherwise poll for one uses it.

A tool that blocks registers what stopping it means: `shell` TERMs its process group and KILLs it after `platform::proc`'s grace period; the gate kills the formatter or the compiler it started; `mcp` kills the group of the server a call waits on, and the next call starts that server again; `web` and `search` end the CDP wait their page sits on, and wake the pause between pages, the wait for a human to clear a verification, and the queue for a permit. A tool that finishes in microseconds registers nothing, and the front door does not pretend otherwise.

The stop is never polled: a wait registers what ending it means, and wakes on that alone.

What a stop leaves behind depends on how far the call got:

| call | history |
|---|---|
| started | one failed result, `[interrupted: the user stopped the turn]` as its last line |
| stopped before it wrote | one failed result naming what is still untouched |
| never started | one failed result saying it never ran |

A reply cut short mid-stream keeps no calls at all: the model never finished asking, so no call ever existed. Once its arguments arrive whole a call belongs to the turn, and every one of them answers — the ones that ran with what they produced, the ones the stop kept from running with the fact that they never ran. A stop ends the work, never the record of it. A call's own deadline is not a stop: `shell` reports `timed out after Ns` through `ToolOutput::failure`.

`final submit` is not a stop either: its result carries `stop`, and the turn settles as `finished`.

### edit and splice

A session gets one of the two, because two ways of locating one change pull the model apart. A profile's `tools.yaml` may name either; naming both keeps `splice` and drops `edit`, with a notice. A profile that names no list gets every tool but `edit` and `final`.

The field each one takes, and its own rules, are in `fs/edit/README.md` and `fs/splice/README.md`.

### Invariants

1. A tool reads only `Env`; no database, no config file, no loop.
2. A tool never measures or trims output; the front door does — `read` and `grep` alone bound their own text, which is why the front door does not measure them. `grep` caps its own hit lines by the same line budget and states every cap that bites, so nothing is spilled or dropped silently.
3. A tool names an effect kind; the front door stamps `call_id` and `ts`.
4. File tools read and write only inside the licensed zones; atomically, under a per-path lock.
5. The roster is the only way the model learns a tool exists.
6. One call runs at a time inside a session; different sessions run concurrently.
7. A tool starts no job and writes no session row; it describes both and the loop acts.
8. The session's list is the whole gate; a tool adds no condition of its own.
9. A stop is one `Stop`, and only a tool that blocks registers what stopping it means: `shell`, the gate, and `mcp`.
10. A call whose arguments arrived whole answers, whether or not it ran.

### Tool manuals

| tool | manual |
|---|---|
| `read` | `fs/read/README.md` |
| `grep` | `fs/grep/README.md` |
| `edit` | `fs/edit/README.md` |
| `splice` | `fs/splice/README.md` |
| `write` | `fs/write/README.md` |
| `cd` | `cd/README.md` |
| `shell`, `alias` | `shell/README.md` |
| `kairos` | `kairos/README.md` |
| `task`, `final` | `subagent/README.md` |
| `mcp` | `mcp/README.md` |
| `web` | `web/README.md` |
| `search` | `search/README.md` |
| `memo`, `todo`, `context` | `memo/README.md` |

## 2. Design intent

- **One table, one front door.** Gates, argument decoding, effect stamping and output measurement happen once, so a new tool cannot forget one of them and cannot report output straight into the conversation.
- **Grouped contract.** The outward shape sits in five modules by caller intent, so a reader looking for "what a tool returns" does not scroll past the ports the loop implements.
- **The list decides.** A profile names what a session may call; the tool layer does not second-guess it. The one exception is settled at spawn, not at call time: a child's list is corrected once, so no check has to be repeated on every call.
- **Positional parameters.** The schema and the parser come from one `&[Param]`, so the JSON schema the model sees cannot drift from what the tool reads.
- **`intent` everywhere.** Every call carries one line of user-facing intention; the front end can show it without parsing arguments.
- **Ports, not dependencies.** Reads the tool layer must not own — memos, artifacts, jobs, children — arrive inside `Env` as traits, so the tools keep no database handle.
- **Effects land with their block.** A memo, alias or sub-session cannot exist without the tool result that caused it; one transaction owns both.

## 3. Unresolved

None at this layer.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.outputMaxLines` | inline line budget, default 120; `read` keeps its own |
| `config.server.agent.tools.outputMaxBytes` | inline byte budget, default 32768; `read` and `grep` keep their own |

Every other key belongs to one tool or one domain and is listed in its own manual.
