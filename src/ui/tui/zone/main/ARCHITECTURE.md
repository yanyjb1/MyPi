# main — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `MainZone`, its fields, and the `Zone` implementation | public |
| `arbitrate.rs` | `Heights`, `arbitrate`, `settle`, `broadcast`, `lend`, cursor position | private |
| `dispatch.rs` | key routing and exit-to-handover conversion | private |
| `requests.rs` | request aggregation | private |
| `keys.rs` | `matches`, the static `TABLE`, `Borrowed` | private |
| `geometry.rs` | input container height, scroll, cursor clamping | private |
| `num/` | compact, exact, and money formatting | private |
| `history/` | `HistoryZone` | private |
| `input/` | `InputZone` | private |
| `reserved/` | `ReservedZone` | private |

## What it exposes

- `MainZone` through `Zone`: delivery, attach/detach, takeover, render, cursor, requests.
- `set_usage_config`, `set_history_config`, `warm`, `cursor_position`.
- `Heights` for callers that need the last division.

## What it calls

- `zone::contract` for `Ctx`, `Handover`, `Request`, `SubZone`, `TermSize`, `Zone`.
- `action::Action` and `event::Event`.
- `theme::Palette` and `history::render::theme::HistoryTheme` while rendering.
- `history::zone::HistoryZone`, `input::InputZone`, `reserved::ReservedZone` directly; they are
  private fields.
- `input::ExitRequest`, `input::pending_hire`, and `reserved::{Hire, Reply}` for the input/reserved
  handshake.

## What calls it

`App` only. It delivers events and actions, reads rows and the cursor, and takes requests.

## What it does not know

The app, the session client, the socket, the database, the agent loop, the picker, and the tree.

## Must stay in sync

1. `arbitrate` and the three sub-zones' `request_height` / `assign` contracts.
2. `keys::TABLE` and the sub-zones' claims.
3. `dispatch::exits` and `ExitRequest`: a new exit needs a destination or a request.
4. `render` order and the cursor: the input's container offset is the gap plus history rows.
5. `take_over` and the memo alias `tree.focus`.
6. `detach` and every sub-zone's `detach`.

## Not wired

1. No loop calls the `Zone` methods.
2. The static key table has no compile-time duplicate check.
3. `HistoryZone::CLAIMS` is a public const; the other claims are reached through `SubZone`.
