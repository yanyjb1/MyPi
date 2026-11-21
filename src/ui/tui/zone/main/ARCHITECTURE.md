# main — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-export | public |
| `zone.rs` | `MainZone`, the bus flush, the directory reader, the `Zone` implementation | public |
| `arbitrate.rs` | `Heights`, `arbitrate`, `settle`, `broadcast`, the cursor's position | private |
| `desk.rs` | `Hire`, `Reply` — what the input asks for and what a hand hands back | private |
| `keys.rs` | `Keys`: the two static seat tables and the keys a hand lends | private |
| `pane.rs` | `Pane`: the row ask, grant and broadcast a sub-zone answers | private |
| `geometry.rs` | input container height, scroll, cursor clamping | private |
| `num/` | compact, exact, and money formatting | private |
| `history/` | `HistoryZone` | private |
| `input/` | `InputZone` | private |
| `reserved/` | `ReservedZone` | private |

## What it talks to

- `zone::contract` for `Zone`, `Claim`, `Kind`, `Outcome`, `TermSize`,
  `Shout`, `name`, `Frame`, `Structure`.
- `ui::tui::action::Action`, and `ui::tui::theme::Accent` for the one live color.
- The three sub-zone crates directly; they are private fields.

## What calls it

`App` only, through the `Zone` surface.

## What it does not know

The app, the session, the socket, the database, the agent loop, the picker, and the tree.

## Inside

```text
MainZone
 ├── history  ── blocks, live tail, the two regions, viewport, products
 ├── input    ── editor, statusline, exits, hires
 ├── reserved ── desk, completion, usage panel, memo panel
 ├── keys     ── seats and loans
 ├── size / heights / shouts
```

- `event` and `action` end in `flush`, which drains every child's `shouts` and delivers them to the
  other two.
- `tick` runs the statusline's own clock, `settle`, the hand that the editor asked for, and the
  history zone's settled dirty set with its asks; then it folds the children's revisions into one
  number the app compares.
- `settle` re-arbitrates rows, seats a pending hand, and flushes again.
- `render` calls each child in row order; the desk reports the cursor's column back to the input.
  The children lay rows only: every model change and every ask happens in `tick` or on an action.

## Must stay in sync

1. `shout::name` and the children that shout or hear each name.
2. `Hire` and the desk's hand table.
3. `arbitrate` and the three children's `pane::Pane`: `request_height` / `assign` / `on_height_changed`.
4. `keys::Keys::new` and the two static seat tables: history's `keys::CLAIMS` and `InputZone::CLAIMS`.
5. `exits` and every `ExitRequest`.
6. `Keys::seat` and the seat order: a lent hand first, then history and input as they were passed.
7. A child's mutating entry point and its `revision`: a change that skips the bump never draws.
8. `HistoryZone::anim` and `Zone::anim`: the ESC row's clock is the only time-driven row here.

## Not wired

Nothing.
