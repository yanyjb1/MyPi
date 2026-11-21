# hub — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-export | public |
| `zone.rs` | `HubZone`, `Row`, `Block`, the `Zone` implementation, `title` | public |
| `render.rs` | the page: head, the note under it, three rows per child, the preview, hints | private |

## What it exposes

- `HubZone` through `Zone`.
- `Row`, `Block`, `title` for the renderer.

## What it calls

- `zone::contract` for `Kind`, `Claim`, `Outcome`, `TermSize`, `Zone`, `ZoneId`, `Structure`.
- `action::{Action, Key, KeyCode, Mods}`.
- `zone::page` for the chassis and for `Picker`.
- `theme::{Accent, fixed}` for the colors, read off the shared palette on every draw.
- `text::bytes` for the transcript size; `lang` and `i18n` for the page's own words.

## Who calls it

`App` only, through the `Zone` surface.

## What it does not know

The other zones, the agent loop, the database, or what a job means.

Which key means what: it decides that itself. It also decides nothing about a child's history
beyond which blocks the structure says exist.

## Must stay in sync

1. `Row::of` and the `call children` answer's columns: `id` alone is required, and every other field
   falls back, so an answer missing one still lists its children.
2. `Block::of` and the shape of a block row; `short_kind` and the wire's block types.
3. `Block` here and the tree's own projection: both write `role: summary`, and the renderer splits on
   the same `": "`.
4. `keys()` and `render::hints`.
5. `anim` and `tick`: the page refetches on that clock and nowhere else.
6. `zone.rs`'s `mine` and the echoed `session` on a `call children` answer: one connection follows
   one conversation, and this page speaks about that one.
7. The preview's `session` check and `call blocks`: a reply without it is discarded, so every answer
   the preview reads names the child it speaks about.

## Not wired

1. Duplicate claims between this page and `resume` are not rejected at boot.