# tui — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list | private |
| `action.rs` | `Action`, `Key`, `KeyCode`, `Mods`, crossterm projections | public |
| `event.rs` | `Event` — runtime and environment signals | public |
| `term.rs` | `Term` — alternate screen, keyboard flags, size, one frame, restore | public |
| `app/` | `App` — registry, ownership, delivery, lifecycle, handover, memo, identity, startup | public |
| `app/mod.rs` | `App` fields and the delivery/render/request surface | public |
| `app/startup.rs` | `Startup` — initial zone, session, working directory | public |
| `app/id.rs` | dependency-free `Uuid` | public |
| `app/memo.rs` | `Memo`, `Operator` | public |
| `app/lifecycle.rs` | attach, detach, session id | public |
| `app/handover.rs` | the handover transaction | public |
| `zone/` | contract and the three major zones | public |
| `zone/contract/` | `Ctx`, `TermSize`, `Zone`, `SubZone`, `Request`, `Handover`, `ZoneId`, `Claim`, `Seat`, `Ownership` | public |
| `zone/main/` | `MainZone` and its private sub-zones | public |
| `zone/main/arbitrate.rs` | row division, height broadcast, cursor position | private |
| `zone/main/dispatch.rs` | key routing and exit-to-handover conversion | private |
| `zone/main/requests.rs` | request aggregation | private |
| `zone/main/keys.rs` | static claims table and runtime loans | private |
| `zone/main/geometry.rs` | input container geometry and cursor clamping | private |
| `zone/main/num/` | compact, exact, and money formatting | private |
| `zone/main/history/` | transcript: local section, viewport, products, layout, cache, painter | private |
| `zone/main/input/` | editor, input history, command route, key semantics, statusline, container | private |
| `zone/main/reserved/` | desk, hand contract, completion, usage panel | private |
| `zone/resume/` | `ResumeZone` — session picker | public |
| `zone/tree/` | `TreeZone` — block tree | public |
| `theme/` | bundled and custom themes, token vocabulary, epoch, repaint hook, palette snapshot | public |
| `text/` | display width, truncation, wrapping | public |

## What it talks to

- **Calls**: `ui::session` for the borrowed view and for what the loop lands; `commands` for the
  command table; `server::config::read::config` for the TUI config shapes; `ratatui` and
  `crossterm` for the screen and input.
- **Is called by**: a frame loop. It does not exist yet.
- **Does not know**: the server process, the database, the socket, the agent loop, or the model.

## Inside

```text
App
 ├── ownership: Ownership      ── which zone shows
 ├── memo: Memo                ── only during handover/takeover
 ├── main: MainZone            ── history + input + reserved
 ├── resume: ResumeZone
 └── tree: TreeZone

MainZone
 ├── history: HistoryZone
 ├── input: InputZone
 ├── reserved: ReservedZone
 ├── keys: Borrowed
 ├── heights: Heights
 ├── pending_exits: Vec<ExitRequest>
 └── pending_requests: Vec<Request>
```

- `App::deliver_event` broadcasts to the showing zone; `App::deliver_action` gives the action to
  that zone and executes any handover it returns.
- `MainZone` resolves the action to a seat, delivers it, then drains the sub-zones' requests and
  exit requests. Screen exits become one `Handover`; the rest are kept for the next `take`.
- `App::render` asks only the showing zone for rows; `App::cursor` asks only `Main`.
- `App::take_requests` returns carried requests first, then the showing zone's own.
- `Term` owns the terminal and draws one `Vec<Line>` plus an optional cursor.

## Must stay in sync

1. `Zone` and `SubZone` and every implementation: one method added is two levels of edits.
2. `Action`'s key identity and `Claim`: a new key kind has to be claimable.
3. `Event` and the loop that will produce it: a variant nobody produces is dead.
4. `Ctx` and every zone that reads it.
5. `Request` and the loop that lands it: a new variant with no landing site is silent work.
6. `App::attach` / `detach` order and the sub-zones' lifecycle: data first, display second.
7. `theme_epoch()` and every cache keyed by it.
8. `TuiConfig` fields and the zones that consume them.

## Not wired yet

1. No frame loop: no event producer, no tick, no resize delivery, no draw, no request landing.
2. `src/lib.rs` has no `pub mod ui;`; `src/ui/mod.rs` does not exist; `Cargo.toml` has no
   `ratatui` or `crossterm`.
3. `main.rs::front_end` bails instead of constructing `App` and `Session`.
4. Session lacks the `Init` call/reply, `tree_range`, and `register` / `unregister`; App identity
   and History's `Init` / Tree's range are dangling.
5. `Startup.session` is not consumed by `App::new`; `App` starts with no session.
6. `App::set_tree_config` has no caller; `cherishYourWords` stays at its default.
7. History's `layout::lazy` is not consumed; `resize::plan` computes a lazy list nobody uses.
