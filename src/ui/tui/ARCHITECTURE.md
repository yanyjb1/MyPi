# tui — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `run.rs` | `run`: the terminal, the loop, the exit path | public |
| `signal.rs` | `Stop`: the four stop signals, polled once a frame | public |
| `config.rs` | `ThemeConfig`, `TodoFold`, `UsageAlign` | public |
| `action.rs` | `Action`, `Key`, `KeyCode`, `Mods`, crossterm projections | public |
| `term.rs` | `Term`, `Input`: alternate screen, flags, size, one frame, one poll | public |
| `app/` | `App`: registry, display ledger, delivery, landing, lifecycle, memo | public |
| `zone/` | `contract`, `registry`, the three major zones | public |
| `theme/` | bundled and custom themes, token vocabulary, epoch | public |
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
 ├── size / theme / attached
 └── finished

MainZone
 ├── history / input / reserved
 └── keys, heights
```

- `App::pump` forwards what the session produced and reconciles the attachment.
- `App::deliver` classifies by `kind`, checks the showing zone's declaration, hands the frame over.
- `App::action` resolves the claim, hands the action over, carries the outcome.
- `App::land` is the only outbound path: it fills in the session id and calls the session.
- `run` owns the terminal; `App` owns the session.

## Must stay in sync

1. `zone::Kind` and `ui::session::Packet`: one upstream class, one declaration.
2. `Action` and `Claim`: a new action kind has to be claimable.
3. `Zone` and every implementation: one method added is four edits.
4. `App::boot` order and every zone's expectations: attach first, start second.
5. `Memo` aliases with the zones that write and read them (`tree.focus` is the only one).
6. `theme_epoch()` and every cache keyed by it.

## Not wired

1. `Main`'s sub-zones still implement the previous surface; `Main` itself is written against the
   current one, so the chat zone compiles only once the sub-zones follow.
2. Suspend and resume: no trigger path, no handling.
