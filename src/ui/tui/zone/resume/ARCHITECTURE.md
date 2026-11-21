# resume — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `zone.rs` | `ResumeZone`, `Row`, `Scope`, the `Zone` implementation, the row helpers | public |
| `render.rs` | the page: head, filter, one block per session, hints, ages | private |

## What it exposes

- `ResumeZone` through `Zone`.
- `Row`, `Scope` for the renderer.

## What it calls

- `zone::contract` for `Kind`, `Claim`, `Outcome`, `TermSize`, `Zone`, `ZoneId`.
- `action::{Action, Key, KeyCode, Mods}`.
- `zone::page` for the chassis: the edge, the head, the filter box, the key hints, the margins and
  the body's row budget.
- `zone::page::Picker` for the stored list: the rows, the filter words, the cursor and the window.
- `text::card` for the rows inside the body, `theme::fixed` for the colors.
- `theme::Accent` for the title, the filter's prompt and every key, read off the shared palette.
- `lang` for the display language and the server's own words; `i18n` through `lang`'s table.

## Who calls it

`App` only, through the `Zone` surface.

## What it does not know

The other zones, the session, the socket, the database, the agent loop, and the terminal.

Which key means what: it decides that itself, and asks the picker only for the small moves a key
implies. The picker in turn knows nothing of sessions, scopes or the two-step delete.

## Must stay in sync

1. `Row::of` and the `call sessions` answer's fields.
2. `Scope` and the `under` field of the list request.
3. `pending_delete`, `failed` and the render's confirming marker: both are said on the block's
   own title row.
4. `start` and `session(None)`: both reset the page.
5. `haystack` and the fields a reader can search for: the page tells the picker what text to
   match, and `render::title` shows the same four fields.
6. `render::hints` and the keys `keys()` claims.
7. `accent` and the panel: the renderer reads the live color off the palette on every draw.
8. `window` in the render and `block_height`: a block may be cut part-way, so the window is counted
   in physical rows and the picker's `settle_scroll` is not used here.

## Not wired

1. The memo column is a count: the list request carries no memo names.