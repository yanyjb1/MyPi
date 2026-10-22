# tree — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `TreeZone` and the `Zone` implementation | public |
| `ask.rs` | `TreeAsk::Range` | public |
| `rows.rs` | `Row` | public |
| `source.rs` | `rows`, `range`, `current`, role and summary | public |
| `layout.rs` | `NodeRef`, `ForkRef`, `RowGeom`, `Placed`, `rows` | public |
| `render.rs` | one row per block, truncation, colors, selection | private |

## What it exposes

- `TreeZone` through `Zone`.
- `begin(rows, current)`, `is_empty`, `set_config(cherish)`.
- `Row` for callers that build or inspect rows.
- `TreeAsk` for a future range request type.

## What it calls

- `ui::session::View` for `shapes`, `blocks`, `chain`, `attached`, and `turn`.
- `ui::session::BlockRef`.
- `server::storage::BlockKind`.
- `zone::{Ctx, Event, Handover, Request, TermSize, Zone, ZoneId}`.
- `action::{Action, KeyCode}`.
- `app::Operator` and the memo alias `tree.focus`.
- `history::render::theme::HistoryTheme`.
- `text::take_width` and `unicode_width` for display columns.

## Who calls it

`App` only:

- `deliver_event`, `deliver_action`, `render`, `take_requests`
- `attach`, `detach`
- `take_over`
- `hand_over`
- `set_config` through `App::set_tree_config`

`MainZone::take_over` reads the `tree.focus` alias and passes it to `HistoryZone::focus`.

## What it does not know

The app's other zones, the session client, the socket, the database, the agent loop, and the
terminal.

## Must stay in sync

1. `Row` fields and `source::rows` / `render::rows`.
2. `source::rows` and the session's shape/block fields.
3. `layout::rows` and `RowGeom` / `render::prefix`.
4. `TreeAsk` and the loop that lands a range.
5. `hand_over` and `MainZone::take_over`: the alias is `tree.focus`, the content is `turn:block`.
6. `set_config` and the `cherish` key mapping in `App`.

## Not wired

1. `TreeAsk` has no consumer; `Request::TreeRange` is pushed directly.
2. `Session::tree_range` does not exist, so the range has no answer.
3. `App::set_tree_config` has no caller.
4. `source::rows` feeds one fork; multi-fork projection has no data source.
5. The loop that delivers events and actions does not exist.
