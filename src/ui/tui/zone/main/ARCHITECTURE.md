# main — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-export | public |
| `zone.rs` | `MainZone`, the bus flush, the directory reader, the `Zone` implementation | public |
| `arbitrate.rs` | `Heights`, `arbitrate`, `settle`, `broadcast`, the cursor's position | private |
| `dispatch.rs` | key routing, the exits that become requests or a handover | private |
| `desk.rs` | `Hire`, `Reply` — what the input asks for and what a hand hands back | private |
| `keys.rs` | `Seat`, `matches`, the static table, `Borrowed` | private |
| `geometry.rs` | input container height, scroll, cursor clamping | private |
| `num/` | compact, exact, and money formatting | private |
| `history/` | `HistoryZone` | private |
| `input/` | `InputZone` | private |
| `reserved/` | `ReservedZone` | private |

## What it talks to

- `zone::contract` for `Zone`, `SubZone`, `Claim`, `Kind`, `Request`, `Outcome`, `TermSize`,
  `Shout`, `name`, `Frame`, `Turns`.
- `ui::tui::action::Action`, and `ui::tui::theme::Accent` for the one live color.
- The three sub-zone crates directly; they are private fields.

## What calls it

`App` only, through the `Zone` surface.

## What it does not know

The app, the session, the socket, the database, the agent loop, the picker, and the tree.

## Inside

```text
MainZone
 ├── history  ── blocks, live tail, local section, viewport, cache
 ├── input    ── editor, statusline, exits, hires
 ├── reserved ── desk, completion, usage panel
 ├── keys     ── seats and loans
 ├── size / heights / shouts
```

- `event` and `action` end in `flush`, which drains every child's `shouts` and delivers them to the
  other two.
- `settle` re-arbitrates rows, seats a pending hand, and flushes again.
- `render` calls each child in row order; the desk reports the cursor's column back to the input.

## Must stay in sync

1. `shout::name` and the children that shout or hear each name.
2. `Hire` and the desk's hand table.
3. `arbitrate` and the three children's `request_height` / `assign`.
4. `keys::TABLE` and the children's own claims (`CLAIMS` on each type).
5. `dispatch` and every `ExitRequest`.

## Not wired

1. `run` still draws every frame whether or not anything arrived.
2. The desk's usage alignment is not read from config.
3. Duplicate claims are resolved by table order, not rejected.
