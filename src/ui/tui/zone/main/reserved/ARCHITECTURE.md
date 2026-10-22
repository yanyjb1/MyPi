# reserved — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `ReservedZone`, `Hire`, the `SubZone` implementation, usage sync | private to `Main` |
| `hand.rs` | `Hand`, `Report`, `Reply`, `IDLE_ROWS` | private |
| `hands/mod.rs` | the roster module list | private |
| `hands/usage.rs` | `UsagePanel` | private |
| `hands/completion/mod.rs` | the completion module list | private |
| `hands/completion/controller.rs` | `CompletionService`, `InputCtx` | private |
| `hands/completion/engine.rs` | `Completion`, `CompletionPopup`, `Scan`, `suggest` | private |
| `hands/completion/popup.rs` | `PopupView`, `render` | private |

## What it exposes

Through `SubZone`:

- `claims` (empty), `borrowed`
- `deliver_event`, `deliver_action`
- `attach`, `detach`
- `request_height`, `assign`, `on_height_changed`
- `take_requests` (empty)

To `Main`:

- `Hire`
- `hire`, `text`, `col`, `take_reply`
- `set_usage_config`, `show_usage`, `sync_usage`
- `render_rows`

## What it calls

- `zone::Ctx`, `Event`, `Request`, `TermSize`, `Claim`.
- `action::{Action, Key}`.
- `api::types::UsageLang` and `api::call::usage::ModelUsage`.
- `config::UsageAlign`.
- `zone::main::num` for compact, exact, and money formatting.
- `theme::Palette`.
- `ui::session::View` for `usage`, `cwd`, and `pools`.

## Who calls it

`MainZone` only:

- `arbitrate` → `request_height`, `assign`
- `settle` / `broadcast` → `on_height_changed`
- `dispatch` → `hire`, `text`, `take_reply`, `deliver_action`
- `deliver_event` → `deliver_event`, `sync_usage`
- `render` → `render_rows`, `col`
- `attach` / `detach`
- `take_requests`

## What it does not know

The app, the history zone, the input zone's internals, the picker, the tree, the socket, the
database, and the agent loop.

## Must stay in sync

1. `Hand` and every hand: a method added is two implementations.
2. `Report` and the desk's `request_height` / `borrowed` / `text` / `col`.
3. `Hire` and `ReservedZone::hand`: a new hand is a roster entry and a match arm.
4. `Reply` and `InputZone::apply_reply`.
5. `sync_usage` and `View.usage`.
6. `set_usage_config` and `UsageConfig`.

## Not wired

1. Completion's pools are not fetched; `view.pools` is the intended source.
2. The loop that calls `deliver_event`, `deliver_action`, and `render` does not exist.
3. The usage panel's absolute/percentage and multi-language views are not implemented.
4. The statusline's bridge mark is not sent by the server.
