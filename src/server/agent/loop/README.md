# loop

## 1. Design / Schema

`loop` orchestrates one turn: build a tool environment, send one request per lap, run the requested tools in order, hand the results back, and close the turn.

Public surface:

- `run(w, r, live, turn, out) -> Result<Ended>`
- `Live`, `Settings`, `Turn`, `Stop`, `Ended`
- `driver::Driver`
- `budget::ToolBudget`
- `outlet::{Outlet, ToolEvent}`

`Live` holds the session-scoped transport, driver, tool roster, stop flags, artifact handle, and settings. `Driver` holds the session cwd and tool budget. `ToolBudget.max = None` means unlimited; `Some(n)` counts non-`final` calls across laps and turns.

`Turn` is `{ session, turn_id }`. `Stop` is `Model`, `User`, `Broken`, or `Killed`. `Ended` carries the stop and the turn's summed `RoundUsage`.

`Outlet` receives tool events, tool output, retry notices, and the one-time budget-exhausted signal. Request content no longer passes through `Outlet`.

`RequestContent` is `{ session, system, tools, max_output_tokens, profile }`. `ToolRun` is `{ call_id, ok, result, memo, spill, duration_ms, effects, stop }`.

`Submit::RoundEnd` is written once per turn with the stop spelling and summed usage. `Submit::Request` is written on the first lap only. `Submit::Session { change: Cwd { turn, path } }` is written when `cd` changes the driver cwd.

## 2. Design Intent

- One session has one `Driver`; tools from one driver run serially, while different sessions can run concurrently.
- A batch uses the cwd snapshot taken at batch start. A successful `cd` updates the driver for the next batch, not the current one.
- The budget counts only non-`final` calls. `final` is always allowed and never charged. Once the budget is exhausted, the roster contains only `final`; other calls get a failed result without execution.
- Budget exhaustion is not a stop. A subagent may continue using `final` or end naturally.
- A `final` result carrying `stop` ends the turn after its result is filed.
- Request writes blocks directly to storage. The loop only passes the writer, retry sink, and cancel flag; it does not decode or re-cut content.
- Tool effects travel with their tool-result blocks in one submission.
- `stop` and `killed` are separate flags so a parent task kill is not reported as a user interrupt.
- Todo state is not held here. The todo tool emits blocks; front ends own replay.
- `Live` belongs to the API layer. The loop borrows it and does not decide when it is dropped.

## 3. Unresolved

- Outside callers still need to construct the new `Live`/`Settings` shapes and implement the new `Outlet`.
- `tools::ToolOutput` must expose `effects` and `stop`; the loop consumes both.
- `config` must expose the subagent budget and request retry keys before the API can fill `Settings`.

## 4. Config Fields

- `config.server.agent.subagent.maxToolCalls` — subagent cumulative non-`final` call cap; default 1000.
- `config.server.agent.request.maxRetries` — retries after the first request.
- `config.server.agent.request.retryInterval` — fixed wait between retries.
- `config.server.agent.request.timeout` — first-byte timeout.
- `config.server.agent.tools.bashTimeout` — bash wall-clock limit.
- `config.server.agent.tools.outputMaxLines` — inline tool-output line cap.
- `config.server.agent.tools.outputMaxBytes` — inline tool-output byte cap.
- `config.server.agent.tools.browser` — browser bundle for web tools.
- `config.server.agent.profile` — active profile name; its tool list gates the roster.
