# contract — architecture

What each file holds and who names it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports, including `Packet` and `Structure` | public |
| `zone.rs` | `Zone`, `SubZone` | public |
| `kind.rs` | `Kind`, `Kind::of` | public |
| `claim.rs` | `Claim`, `Claim::of` | public |
| `request.rs` | `Request`, `ModelId` | public |
| `outcome.rs` | `ZoneId`, `Outcome` | public |
| `size.rs` | `TermSize` | public |
| `memo.rs` | `Memo` | public |
| `shout.rs` | `Shout`, `Origin`, `name` | public |

## What it talks to

- `action::{Action, Key}` for the claim projection.
- `ui::session` for `Packet` and `Structure` — re-exported, so no zone imports session.
- `ratatui::text::Line` for `Zone::render`.
- `serde_json::Value` for what a shout carries and what `hear` reads.

## What calls it

`app` and every major zone; `Main` and its sub-zones for the second level.

## Must stay in sync

1. `Kind` and the upstream frame classes.
2. `Claim` and `Action`: a new action kind is a new claim.
3. `Zone` and its three implementations.
4. `Outcome` and `App::settle`.
5. The re-exports and what zones actually import: a type zones need must be listed here.
6. `shout::name` with every zone that shouts or hears a name.

## Not wired

1. Duplicate claims are not rejected at boot.
2. `SubZone`'s row handshake is provisional.
