# app — architecture

Where the app sits, what each file holds, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `App`, its fields, and the delivery/render/request surface | public |
| `startup.rs` | `Startup` and its two constructors | public |
| `id.rs` | `Uuid`, `new`, `as_bytes`, `Display` | public |
| `memo.rs` | `Memo`, `Operator`, `call`, `clear` | public |
| `lifecycle.rs` | `session`, `attach`, `detach` | public |
| `handover.rs` | `hand_over` and the debug assertion | public |

## What it exposes

- `App::new(startup, cfg)` — assemble zones, initialize the theme, apply history and usage
  config, and open `Resume` when the startup asks for it.
- `id`, `born`, `died` — the front end's identity.
- `set_tree_config` — the tree's `cherishYourWords` switch.
- `showing` — the current `ZoneId`, or `None`.
- `deliver_event`, `deliver_action`, `render`, `cursor`, `take_requests`.
- `attach`, `detach`, `session`.
- `hand_over`.

## What it calls

- `zone::MainZone`, `ResumeZone`, `TreeZone` through `Zone` only.
- `zone::Ownership` for the screen ledger.
- `zone::Ctx` for every delivery.
- `theme::init_from_config` at assembly.
- `MainZone::set_usage_config`, `set_history_config`, and `warm`.
- `ResumeZone::begin` when the initial zone is `Resume`.

## What calls it

A frame loop. It does not exist yet.

## What it does not know

The terminal, crossterm, the socket, the session client, the database, the agent loop, and the
zones' internal fields.

## Must stay in sync

1. `Startup` and the CLI's launch parameters.
2. `App::attach` order and every zone's `attach` expectations: data first, display second.
3. `App::hand_over` and the `Zone` contract: old `hand_over`, carry, register, new `take_over`.
4. `Memo` aliases with the zones that read them; `tree.focus` is the only alias in use.
5. `App::new` and the `TuiConfig` fields it consumes.

## Not wired

1. No loop constructs `App`, delivers events, draws, or lands requests.
2. `Startup.session` is not used after construction.
3. `Uuid` is not registered with the session.
4. `set_tree_config` has no caller.
5. `src/lib.rs` does not declare `ui`; the app is not in the build.
