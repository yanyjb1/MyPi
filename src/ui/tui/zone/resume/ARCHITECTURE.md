# resume — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `ResumeZone`, `Scope`, the `Zone` implementation, row helpers | public |
| `render.rs` | the page: frame, header, search, list, footer | private |

## What it exposes

- `ResumeZone` through `Zone`.
- `begin(cwd)` — open the page and ask for the list.
- `sync(view)` — recompute the visible set when the row count moved.
- `selected_id(view)` — the selected session, through the filter.
- `Scope` and the display helpers used by the renderer.

## What it calls

- `ui::session::View` for `rows`, `cwd`, `notices`, and `refused`.
- `api::call::sessions::SessionRow` and `display_name`.
- `platform::time::age_seconds`.
- `zone::{Ctx, Event, Handover, Request, TermSize, Zone, ZoneId}`.
- `action::{Action, KeyCode}`.
- `history::render::cards` and `HistoryTheme` for the card frame and colors.

## Who calls it

`App` only:

- `deliver_event`, `deliver_action`, `render`, `take_requests`
- `attach`, `detach`
- `take_over`
- `begin` at startup when `Startup.initial` is `Resume`

## What it does not know

The app's other zones, the session client, the socket, the database, the agent loop, and the
terminal.

## Must stay in sync

1. `Scope` and `Request::List { under }`: the scope decides the directory filter.
2. `selected_id` and `visible`: the visible index is not the row index.
3. `pending_delete` and the render's confirming marker.
4. `begin` and `take_over`: both open the page and ask for a list.
5. `detach` and the cleared list/filter/selection state.

## Not wired

1. `Request::List`, `Request::Attach`, and `Request::Delete` have no loop.
2. `App` starts with `session = None`; a startup session is not attached.
3. The first frame can render nothing because `size` is `None` until a delivery.
4. `view.refused` is drawn, but there is no dedicated retry path after a refused delete.
