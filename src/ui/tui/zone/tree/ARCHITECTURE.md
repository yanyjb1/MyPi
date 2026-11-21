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
| `render.rs` | the page: head, filter, one row per block, hints | private |

## What it exposes

- `TreeZone` through `Zone`.
- `Row` for callers that inspect the projection.

## What it calls

- `zone::contract` for `Zone`, `Kind`, `Claim`, `Outcome`, `Structure`, `TermSize`.
- `action::{Action, Key, KeyCode, Mods}`.
- `zone::page` for the chassis: the edge, the head, the filter box, the key hints, the margins and
  the body's row budget.
- `zone::page::body_rows` from the zone's own side effects, so the ask covers what the draw shows.
- `lang` for the display language; `i18n` through `lang`'s own table.
- `theme::fixed`, `theme::Accent` and `text::take_width` while drawing; the title, the filter's prompt, the keys and the user node take the live color.
- `unicode_width` for display columns.

## Who calls it

`App` only, through the `Zone` surface.

## What it does not know

The other zones, the session, the socket, the database, the agent loop, and the terminal.

## Must stay in sync

1. `Row` fields and `source::rows` / `render::paint`.
2. `Node::of` and the block row's field names.
3. `layout::rows` and `RowGeom` / `render::prefix`.
4. The memo alias `tree.focus` and `Main::start`.
5. `only_user` and the filter in `source::rows`.
6. `visible`, `row_at` and the draw: the on-screen position is not the row index.
7. `render::hints` and the keys `keys()` claims, including the two that follow `cherish`.
8. `accent` and the panel: the renderer reads the live color off the palette on every draw.

## Not wired

1. `source::rows` feeds one fork; multi-fork projection has no data source.
2. A block the window did not carry is drawn without a summary.