# contract — architecture

What each file holds, who implements it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the re-exports | public |
| `ctx.rs` | `Ctx`, `TermSize` | public |
| `zone.rs` | `Zone`, `SubZone` | public |
| `request.rs` | `Request` | public |
| `handover.rs` | `Handover`, `ZoneId` | public |
| `claim.rs` | `Claim`, `Seat` | public |
| `ownership.rs` | `Ownership` | public |

## Who implements it

- `Zone`: `MainZone`, `ResumeZone`, `TreeZone`.
- `SubZone`: `HistoryZone`, `InputZone`, `ReservedZone`.
- `Ownership`: held by `App`.
- `Claim`: declared by each sub-zone as a `const` or a method return.

## What it calls

- `ui::tui::action::{Action, Key}` for input.
- `ui::tui::event::Event` for broadcasts.
- `ui::tui::app::Memo` for the handover window.
- `ui::session::View` for the read model.
- `ui::tui::zone::main::history::ask::Ask` for `Request::View`.
- `ratatui::text::Line` for rows.

## What calls it

- `App` for the `Zone` surface and `Ownership`.
- `MainZone` for the `SubZone` surface.
- Every zone for `Ctx`, `Request`, `Handover`, `Claim`, and `TermSize`.

## What it does not know

The terminal, crossterm, the socket, the session client, the database, the agent loop, and any
zone's internal state.

## Must stay in sync

1. `Zone` and every major zone: a method added is three implementations.
2. `SubZone` and every sub-zone: a method added is three implementations.
3. `Request` and the loop that lands it.
4. `Handover` and `App::hand_over`.
5. `Claim` and `Action`: every claim class must have a matching predicate.
6. `Ctx` and every delivery site.

## Not wired

1. `Request` has no landing loop.
2. `Request::TreeRange` has no session-side receiver.
3. `ZoneId` has no fourth variant for a future front end; adding one is a compile error in the
   app's three matches, which is intended.
