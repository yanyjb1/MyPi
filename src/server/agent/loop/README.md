# loop

## 1. Design / Schema

`loop` runs one turn: build a tool environment, send one request per lap, run the requested tools in order, hand the results back, and close the turn. It also holds the per-session runtime between turns.

Public surface:

- `run(w, r, live, turn, out) -> Result<Ended>`
- `Live`, `Settings`, `Turn`, `Stop`, `Ended`
- `driver::Driver`
- `budget::ToolBudget`
- `outlet::{Outlet, ToolEvent, Event, EventOutlet}`
- `register::Register`: `new(writer, cfg, events)`, `deliver(session, msg) -> Result<Option<i64>>`,
  `interrupt(session)`, `drop(id)`, `set(session, field, value)`

`Register` books a session shell on first use. `deliver` runs one message; `Some(turn)` is the
round it opened, `None` means nothing was opened (a queued wake). `set` is the one state change:
`profile`, `model`, or `cwd`.

Live lines that never become blocks — retry attempts and budget exhaustion — leave the loop as
`outlet::Event` on the sender handed to `Register::new`. A tool's live output does not go this way:
the driver hands it to the writer as a relay-only piece, so it travels with the content path.

`Live` holds the session-scoped transport, driver, tool roster, stop flags, artifact handle, and
settings. `Driver` holds the session cwd and tool budget. `ToolBudget.max = None` means unlimited;
`Some(n)` counts non-`final` calls across laps and turns.

`Turn` is `{ session, turn_id }`. `Stop` is `Model`, `User`, `Broken`, or `Killed`. `Ended` carries
the stop and the turn's summed `RoundUsage`.

`Outlet` receives tool events, tool output, retry notices, and the one-time budget-exhausted signal;
`EventOutlet` forwards the retry notices and the budget signal, and drops the rest.

`RequestContent` is `{ session, system, tools, max_output_tokens, profile }`. `ToolRun` is
`{ call_id, ok, result, memo, spill, duration_ms, effects, stop }`.

`Submit::RoundEnd` is written once per turn with the stop spelling and summed usage.
`Submit::Request` is written on the first lap only. `Submit::Session { change: Cwd { turn, path } }`
is written when `cd` changes the driver cwd.

## 2. Design Intent

- One session has one `Driver`; tools from one driver run serially, while different sessions run
  concurrently.
- A batch uses the cwd snapshot taken at batch start. A successful `cd` updates the driver for the
  next batch, not the current one.
- The budget counts only non-`final` calls. `final` is always allowed and never charged.
- A `final` result carrying `stop` ends the turn after its result is filed.
- The todo workbench is rebuilt from the session's blocks at every turn and discarded at the end, so
  a rewind never leaves a stale list.
- The profile only lands at a session's first turn or the first turn after a compaction; later turns
  keep what the request archive and the compaction handoff carry.
- `stop` and `killed` are separate flags so a parent task kill is not reported as a user interrupt.
- A subagent is simply another registered session; this loop never starts one and holds no handle
  to one.

## 3. Unresolved

- Nothing starts a subagent session yet: the tools layer has no path back into `register`.
- `set("model")` swaps the live transport; if no `Live` exists the next build reads the row.

## 4. Config Fields

- `config.server.agent.request.maxRetries` — retries after the first request.
- `config.server.agent.request.retryInterval` — fixed wait between retries.
- `config.server.agent.request.timeout` — first-byte timeout.
- `config.server.agent.tools.bashTimeout` — bash wall-clock limit.
- `config.server.agent.tools.outputMaxLines` — inline tool-output line cap.
- `config.server.agent.tools.outputMaxBytes` — inline tool-output byte cap.
- `config.server.agent.tools.browser` — browser bundle for web tools.
- `config.server.agent.profile` — active profile name; its tool list gates the roster.
