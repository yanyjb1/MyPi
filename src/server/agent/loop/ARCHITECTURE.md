# loop architecture

## Files

```text
mod.rs          run, Live, Settings, Turn, Stop, Ended
driver.rs       Driver: cwd, serial tool calls, cwd persistence
budget.rs       ToolBudget: cumulative count, final exception, exhaustion
round.rs        one lap: request, tools, results
env.rs          Env build and context window
artifacts.rs    artifact fetch and temp-file lifetime
outlet.rs       Outlet, ToolEvent, RetryToOutlet
```

## Relations

- `run` builds one `Env`, loops over laps, sums usage, and writes `RoundEnd`.
- `run` asks `round::once` for a lap. `round::once` builds `RequestContent`, calls `request::start`, records the first request archive, and runs tools through `Driver`.
- `request::start` writes `Submit::Blocks` itself and returns `Answer`. The loop passes `RetryToOutlet` and the session stop flag.
- `Driver::run_calls` applies the budget, executes tools serially, emits `ToolEvent`, and calls `request::hand_results` through `round::once`.
- A successful `cd` updates the driver cwd and writes `SessionChange::Cwd`; the current batch keeps its snapshot.
- `ToolRun.effects` are handed to `request::hand_results`, which puts them in the same `Submit::Blocks` as the tool results.
- `env::build` gives tools the driver cwd, output limits, browser, bash settings, profile gate, context window, and artifact handle.
- `env::build` reads `cwd_history` and `request::rebuild`; it does not write.
- `ArtifactStore` owns the session's temp files; dropping `Live` drops the handle and cleans them.
- `budget::ToolBudget` is held by `Driver`; `run` uses it to filter the roster and to send the one-time exhaustion notice.
- `outlet::RetryToOutlet` adapts `Outlet` to request's `RetrySink`; the loop does not interpret retry notices.
- `loop` does not parse config, open the database, decode protocols, or write blocks.
- `loop` does not know API subscriptions, front-end rendering, todo replay, or tool implementations.

## Callers

- The API layer builds `Live`, owns it between turns, and calls `run`.
- The API implements `Outlet` for its live channel.
- Storage supplies the read connection and consumes submissions.
- Request supplies `Client`, `RequestContent`, `ToolRun`, and `hand_results`.
- Tools supply `Env`, `Workspace`, `ToolOutput`, and execution.
