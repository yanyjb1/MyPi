# history — architecture

Where the zone sits, what each file holds, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list | private |
| `zone.rs` | `HistoryZone`, `CLAIMS`, the frame pass, the asks it owes | private to `Main` |
| `shape.rs` | `Block`, `Kind`, `Usage`: one row decoded | private |
| `live.rs` | `Live`: the running turn's blocks, gathered from pieces | private |
| `local.rs` | `Local`, `Slot`, `SlotKind`, `Level`, `Life` | private |
| `view.rs` | `Viewport`, `Anchor`, `Dir`, the walk | private |
| `segment.rs` | `Segment`, `Kind`, `split` — the source split by line | private |
| `cache/` | `BlockCache`: rows per slot and their keys | private |
| `render/` | painting: `block`, `cards`, `notice`, `text` | private |

## What it exposes

Through `SubZone`: `claims`, `kinds`, `deliver_event`, `deliver_action`, `attach`, `detach`,
`structure`, `request_height`, `assign`, `on_height_changed`, `take_shouts`, `hear`.

To `Main`: `notice`, `focus`, `reset`, `render_rows`.

## What it calls

- `ui::tui::zone::{Frame, Turns, Request, Shout, name, Claim, Kind, SubZone, TermSize}`.
- `ui::tui::TodoFold`, `ui::tui::theme::{HistoryTheme, Token, theme_epoch}`.
- `text` for display width and wrapping.
- `render::cards::tools` for the tool renderer table.

## Who calls it

`MainZone` is the only caller: `arbitrate` → `assign`, `settle` → `on_height_changed`, `render` →
`render_rows`, `dispatch` → `deliver_action`, `event` → `deliver_event`, `structure` → `structure`,
the bus → `hear` and `take_shouts`.

## What it does not know

The app, the input zone, the desk, the picker, the tree, the socket, the database, and the agent
loop. Every ask leaves as a `Request`; every fact for another zone leaves as a `Shout`.

## Must stay in sync

1. `shape::Kind` and the wire's `type` column.
2. `local::SlotKind` and the walk's height/separator rules.
3. `segment::Kind` and the painter's per-segment branch.
4. `cache::sync` keys with `theme_epoch`, width, working directory and the fold switches.
5. `shout::name` values with the names this zone shouts.
6. `structure()` and the local section: a slot lives while the structure owns its block.
7. `CLAIMS` and the key handling in `deliver_action`.

## Not wired

1. `ask_turn` runs when the anchor's round and its neighbours are missing, but nothing asks for a
   round whose rows the walk only partly reached.
2. The cut-preserving re-layout on a resize is not built: `Viewport::on_resize` clamps the row.
3. A live block is painted whole on every piece.
