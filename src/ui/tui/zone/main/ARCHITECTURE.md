# main — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-export | public |
| `zone.rs` | `MainZone` and the `Zone` implementation | public |
| `arbitrate.rs` | `Heights`, `arbitrate`, `settle`, `broadcast`, `lend`, cursor position | private |
| `dispatch.rs` | key routing and exit-to-request conversion | private |
| `keys.rs` | `Seat`, `matches`, the static `TABLE`, `Borrowed` | private |
| `geometry.rs` | input container height, scroll, cursor clamping | private |
| `num/` | compact, exact, and money formatting | private |
| `history/` | `HistoryZone` | private |
| `input/` | `InputZone` | private |
| `reserved/` | `ReservedZone` | private |

## What it exposes

- `MainZone` through `Zone`.

## What it calls

- `zone::contract` for `Zone`, `SubZone`, `Claim`, `Kind`, `Request`, `Outcome`, `TermSize`.
- `action::Action` and `ui::session::{Packet, Structure}`.
- `history::zone::HistoryZone`, `input::InputZone`, `reserved::ReservedZone` directly; they are
  private fields.

## What calls it

`App` only, through the `Zone` surface.

## What it does not know

The app, the session, the socket, the database, the agent loop, the picker, and the tree.

## Must stay in sync

1. `arbitrate` and the three sub-zones' `request_height` / `assign` contracts.
2. `keys::TABLE` and the sub-zones' claims.
3. `dispatch` and every `ExitRequest`: a new exit needs a destination or a request.
4. `render` order and the cursor: the input's container offset is the gap plus history rows.
5. `start` and the memo alias `tree.focus`.

## Not wired

1. The three sub-zones still implement the previous surface: `Event`, `Ctx`, and a session view.
   `Main` is written against the current one, so the chat zone does not compile until they are
   rewritten.
2. The static key table has no compile-time duplicate check.
