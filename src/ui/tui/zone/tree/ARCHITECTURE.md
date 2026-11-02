# tree — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `zone.rs` | `TreeZone`, `Node`, the `Zone` implementation | public |
| `rows.rs` | `Row` | public |
| `source.rs` | `rows`: the structure and the blocks joined, then laid out | private |
| `layout.rs` | `NodeRef`, `ForkRef`, `RowGeom`, `Placed`, `rows` | private |
| `render.rs` | one row per block, truncation, colors, selection | private |

## What it exposes

- `TreeZone` through `Zone`.
- `Row` for callers that inspect the projection.

## What it calls

- `zone::contract` for `Zone`, `Kind`, `Claim`, `Outcome`, `Request`, `Structure`, `TermSize`.
- `action::{Action, Key, KeyCode, Mods}`.
- `theme::HistoryTheme` and `text::take_width` while drawing.
- `unicode_width` for display columns.

## Who calls it

`App` only, through the `Zone` surface.

## What it does not know

The other zones, the session, the socket, the database, the agent loop, and the terminal.

## Must stay in sync

1. `Row` fields and `source::rows` / `render::rows`.
2. `Node::of` and the block row's field names.
3. `layout::rows` and `RowGeom` / `render::prefix`.
4. The memo alias `tree.focus` and `Main::start`.
5. `only_user` and the filter in `source::rows`.

## Not wired

1. `source::rows` feeds one fork; multi-fork projection has no data source.
2. A block the window did not carry is drawn without a summary.
