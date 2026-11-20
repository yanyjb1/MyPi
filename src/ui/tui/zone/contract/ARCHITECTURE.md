# contract — architecture

What each file holds and who names it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `zone.rs` | `Zone`, `SubZone` | public |
| `kind.rs` | `Kind`, `Kind::of` | public |
| `claim.rs` | `Claim`, `Claim::of`, `matches` | public |
| `mail.rs` | `Mail`, the post box | public |
| `local.rs` | `Local`, the asks the screen alone answers | public |
| `shared.rs` | `Shared`, what every zone is handed at boot | public |
| `outcome.rs` | `ZoneId`, `Outcome` | public |
| `size.rs` | `TermSize` | public |
| `memo.rs` | `Memo` | public |
| `shout.rs` | `Shout`, `name` | public |

`Frame` is the wire's and `Structure` is the session's; both are re-exported here so a zone reads what it is handed without reaching past this file.

## What it talks to

- `action::{Action, Key}` for the claim projection.
- `ratatui::text::Line` for `Zone::render`.
- `serde_json::Value` for what a shout carries and what `hear` reads.
- `ipc::contract` for `Frame`, and `ui::session` for `Structure`.
- `theme::Palette` for the one shared color.

## What calls it

`app` and every major zone; `Main` and its sub-zones for the second level.

## Must stay in sync

1. `Kind` and the upstream frame kinds.
2. `Claim` and `Action`: a new action kind is a new claim.
3. `Zone` and its three implementations.
4. `Outcome` and `App::settle`.
5. The re-exports and what zones actually import: a type zones need must be listed here.
6. `shout::name` with every zone that shouts or hears a name.
7. `ZoneId` and `registry::zones`: a slot's index is its `ZoneId::index`.

## Not wired

1. Duplicate claims are not rejected at boot.
