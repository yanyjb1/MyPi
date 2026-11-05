# reserved — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `ReservedZone`: the seat, the frozen report, the two frames it decodes | private |
| `hand.rs` | `Hand`, `Report`, `IDLE_ROWS` | private |
| `pools.rs` | `Pools`: model addresses and profile names, decoded from `call pools` | private |
| `hands/completion/` | `controller` (the hand), `engine` (candidates), `popup` (drawing) | private |
| `hands/usage.rs` | `UsagePanel`: the `/usage` table | private |

## What it exposes

Through `SubZone`: `claims`, `kinds`, `borrowed`, `deliver_event`, `deliver_action`, `attach`,
`detach`, `hear`, plus `super::pane::Pane`'s `request_height` and `assign`.

To `Main`: `hire`, `text`, `col`, `take_reply`, `take_requests`, `render_rows`.

To `Input`: nothing directly; the main zone carries `Reply` across.

## What it calls

- `ui::tui::zone::{Frame, Request, TermSize, Claim, Kind, name}` and
  `super::desk::{Hire, Reply}`.
- `ui::tui::action` for keys, `ui::tui::theme::Accent` for the accent.
- `ui::command` for `Candidate`; the command table itself belongs to the input zone.
- `crate::git` is not called here; the statusline owns that.

## Who calls it

`MainZone` only: `arbitrate` → `request_height` / `assign`, `dispatch` → `deliver_action`, `event` →
`deliver_event`, `render` → `render_rows`, `deliver_hires` → `hire` / `text` / `take_reply`, the bus
→ `hear`.

## What it does not know

The history zone, the session, the editor's text (it is fed), the socket, the database, the agent
loop, the picker, and the tree.

## Must stay in sync

1. `Hire` with `ReservedZone::hand`'s match.
2. `Report.keys` with the keys the hand actually reads.
3. `Pools` with the `call pools` answer, asked on every completion hire.
4. The completion hand's `ready` / `stays` with the desk's report freeze: names land before a height
   is taken, and a hand still waiting holds the seat.
5. `UsagePanel::take` with the `call usage` answer.

## Not wired

1. `/usage`'s alignment is not read from config.
