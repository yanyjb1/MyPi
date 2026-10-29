# loop architecture

## Files

```text
mod.rs              run, Live, Settings, Turn, Stop, Ended
driver.rs           Driver: cwd, serial tool calls, cwd persistence
budget.rs           ToolBudget: cumulative count, final exception, exhaustion
round.rs            one lap: request, tools, results
env.rs              Env build, todo workbench, context window
artifacts.rs        artifact fetch and temp-file lifetime
outlet.rs           Outlet, ToolEvent, RetryToOutlet
register/mod.rs     Register, Handle, Slot: deliver, interrupt, drop, set
register/build.rs   Live assembly from the stored row
register/compact.rs the compaction round
register/outlets.rs silent outlets for wakes
```

## Relations

- `run` builds one `Env`, loops over laps, sums usage, and writes `RoundEnd`.
- `run` asks `round::once` for a lap. `round::once` builds `RequestContent`, calls `request::start`,
  records the first request archive, and runs tools through `Driver`.
- `request::start` writes `Submit::Blocks` itself and returns `Answer`; the loop passes
  `RetryToOutlet` and the session stop flag.
- `Driver::run_calls` applies the budget, executes tools serially, emits `ToolEvent`, and calls
  `request::hand_results` through `round::once`.
- A successful `cd` updates the driver cwd and writes `SessionChange::Cwd`.
- `env::build` gives tools the driver cwd, limits, browser, bash settings, profile gate, context
  window, and artifact handle. It rebuilds the todo workbench from the session's blocks every turn.
- `register::deliver` opens a round (submit, compact, wake), spawns its thread, and returns the round
  id. `set` moves the live transport, the driver cwd, or a pending profile.
- A turn's outlet is `EventOutlet`: retry notices and budget exhaustion leave through the sender
  given to `Register::new`. Tool output goes to the writer as a relay-only piece instead.
- `register/build.rs` assembles `Live` from the session row and lands a pending profile when due.
- `register/compact.rs` plans and runs a compaction; the loop owns the summary text.
- `ArtifactStore` owns the session's temp files; dropping `Live` drops the handle and cleans them.
- `loop` does not parse config, open the database, decode protocols, or write blocks.
- `loop` does not know subscriptions, front-end rendering, subagents, or tool implementations.

## Callers

- The API calls `Register::deliver`, `interrupt`, `drop`, and `set`; it never touches `Live`.
- Storage supplies the read connection and consumes submissions.
- Request supplies `Client`, `RequestContent`, `ToolRun`, and `hand_results`.
- Tools supply `Env`, `Workspace`, `ToolOutput`, and execution.
