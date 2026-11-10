# tui — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `run.rs` | `run`: the terminal, the loop, the exit path | public |
| `signal.rs` | `Stop`: the four stop signals, polled once a frame | public |
| `config.rs` | `ThemeConfig`, `TodoFold` | public |
| `action.rs` | `Action`, `Key`, `KeyCode`, `Mods`, crossterm projections | public |
| `term.rs` | `Term`, `Input`: alternate screen, flags, size, one frame, drain and wait | public |
| `app/` | `App`: registry, display ledger, delivery, landing, lifecycle, memo | public |
| `zone/` | `contract`, `registry`, the three major zones | public |
| `theme/` | the live accent and the fixed palette | public |
| `text/` | display width, truncation, wrapping, the card frame | public |

## What it talks to

- **Calls**: `ui::session` for the connection and the structural window; `commands` for the
  command table; `crate::git` for the statusline.
- **Is called by**: `ui::tui::run`, from `main::front_end_with`.
- **Does not know**: the server process, the database, the socket, the agent loop, or the model.

## Inside

```text
App
 ├── session: Session
 ├── zones: Vec<Box<dyn Zone>>   ── Main, Resume, Tree
 ├── owner: Option<ZoneId>
 ├── memo: Memo
 ├── size / accent / attached
 └── finished

MainZone
 ├── history / input / reserved
 └── keys, heights
```

- `App::pump` forwards what the session produced and reconciles the attachment.
- `App::deliver` classifies by `kind`, checks the showing zone's declaration, hands the frame over.
- `App::action` resolves the claim, hands the action over, carries the outcome.
- `App::land` is the only outbound path: it fills in the session id and calls the session.
- `App::tick` runs the showing zone's side effects once a round and lands what it asks for.
- `App::frame` draws only when the showing zone's `revision` moved or its `anim` fell due; otherwise
  it returns `None` and the round ends without composing a screen.
- `run` owns the terminal; `App` owns the session.

One round, in order: stop signals and the size, every terminal event already queued, the session's
frames, the zone's tick, a draw if the frame says so, then a wait for the terminal.

## Must stay in sync

1. `zone::Kind` and the app's frame translation: one upstream class, one declaration.
2. `Action` and `Claim`: a new action kind has to be claimable.
3. `Zone` and every implementation: one method added is four edits.
4. `App::boot` order and the zone that shows: attach first, start second.
5. `Memo` aliases with the zones that write and read them (`tree.focus` is the only one).
6. The accent broadcast and every painter that resolves it.
7. A zone's mutating entry points and its `revision`: a change that skips the bump never draws.
8. `Zone::anim` and the row a clock drives: a time-driven row needs a period, or it freezes when
   nothing else changes.

## Not wired

1. Suspend and resume: no trigger path, no handling.
2. The main zone's bus is the only cross-sub-zone channel; there is no other.
