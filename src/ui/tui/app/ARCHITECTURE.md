# app — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `app.rs` | `App`, `Input`, `Accent`, boot, delivery, landing, handover | public |
| `lifecycle.rs` | `attach`, `detach` | public |
| `startup.rs` | `Startup` and its three constructors | public |

## What it exposes

- `App::boot(session, size, startup, zones)`.
- `finished`, `size`, `pump`, `input`, `render`, `cursor`, `bye`.
- `attach`, `detach`.
- `Startup::main`, `Startup::resume`, `Startup::attach`.

## What it calls

- `ui::session::Session`, and `zone::{Frame, Turns}` for what it hands down.
- `zone::{Kind, Claim, Memo, Outcome, Request, TermSize, Zone, ZoneId}`.
- `theme::Accent` for the one live color.

## What calls it

`ui::tui::run`, the frame loop in `run.rs`.

## What it does not know

The terminal, crossterm, the socket, the database, the agent loop, and the zones' internal
fields.

## Must stay in sync

1. `Kind::of` and the upstream frame names it classifies.
2. `App::boot` order and the zone that shows: attach first, start second.
3. `with_session` and the doors that do not take a session (`config`, `pools`, `sessions`).
4. `create`'s payload and api's `post init` arguments.
5. `Startup` and the CLI's launch parameters.

## Not wired

1. A refused `post init` drops the waiting message; the reader has to type it again.
