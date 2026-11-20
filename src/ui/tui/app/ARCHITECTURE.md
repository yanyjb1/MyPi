# app — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `app.rs` | `App`, `Input`, boot, delivery, mailing, handover | public |
| `lifecycle.rs` | `attach`, `detach` | public |
| `startup.rs` | `Startup` and its three constructors | public |

## What it exposes

- `App::boot(session, size, startup, zones)`.
- `finished`, `pump`, `tick`, `frame`, `input`, `cursor`, `bye`, `take_copy`.
- `attach`, `detach`.
- `Startup::main`, `Startup::resume`, `Startup::attach`.

## What it calls

- `ui::session::Session`, and `zone::{Frame, Structure}` for what it hands down.
- `zone::{Kind, Claim, Local, Mail, Memo, Outcome, Shared, TermSize, Zone, ZoneId}`.
- `theme::Palette` for the one shared color.

## What calls it

`ui::tui::run`, the frame loop in `run.rs`.

## What it does not know

The terminal, crossterm, the socket, the database, the agent loop, the doors' payloads, and the zones' internal fields.

## Must stay in sync

1. `Kind::of` and the upstream frame names it classifies.
2. `App::boot` order and the zone that shows: bind first, attach second, start third.
3. `Mail::at` and `registry::zones`: the i-th slot of the registry is the i-th `ZoneId`.
4. `create`'s payload and api's `post init` arguments.
5. `Startup` and the CLI's launch parameters.
6. The `post init` answer and `created`: the app's one letter it reads itself.

## Not wired

1. The queue holds one place per answered letter; a letter the server never answers would shift every later answer by one. Only the watch and the sign-off are silent, and neither enters the queue.
