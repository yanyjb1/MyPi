# resume — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `zone.rs` | `ResumeZone`, `Row`, `Scope`, the `Zone` implementation, the row helpers | public |
| `render.rs` | the page: frame, header, search, list, footer | private |

## What it exposes

- `ResumeZone` through `Zone`.
- `Row`, `Scope` for the renderer.

## What it calls

- `zone::contract` for `Kind`, `Claim`, `Outcome`, `Request`, `TermSize`, `Zone`, `ZoneId`.
- `action::{Action, Key, KeyCode, Mods}`.
- `text::card` for the card frame, `theme::fixed` for the colors.

## Who calls it

`App` only, through the `Zone` surface.

## What it does not know

The other zones, the session, the socket, the database, the agent loop, and the terminal.

## Must stay in sync

1. `Row::of` and the `call sessions` answer's fields.
2. `Scope` and the `under` field of the list request.
3. `pending_delete` and the render's confirming marker.
4. `start` and `session(None)`: both reset the page.
5. The visible index and `row_at`: the on-screen position is not the row index.

## Not wired

1. The preview, the size and the age have no source yet; see the README.
2. A refused delete is drawn as one fixed word.
