# input — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `InputZone`, `CLAIMS`, `ExitRequest`, the frame answers it decodes | private |
| `ctx.rs` | `Ctx`, `Data`, `Money` — what the parts read | private |
| `editor/` | the text, undo, paste store, separators, submission ring | private |
| `semantics.rs` | key → intent, one context-free table | private |
| `route.rs` | one line classified as a message or a command | private |
| `command/` | the table (`table.rs`), the answer shapes (`call.rs`), one file per command | private |
| `render.rs` | the container: top edge, body rows, bottom edge, cursor | private |
| `statusline/` | the row engine, `StatusTheme`, and the eight decos | private |

## What it exposes

Through `SubZone`: `claims`, `kinds`, `deliver_event`, `deliver_action`, `attach`, `detach`,
`take_shouts`, `hear`, plus `super::pane::Pane`'s `request_height` and `assign`.

To `Main`: `take_hire`, `take_exits`, `draft`, `knows_cwd`, `apply_reply`, `clear`, `render_rows`,
`cursor_row`, `cursor_col`, `editor`, `statusline`.

## What it calls

- `ui::tui::zone::{Frame, Request, TermSize, Claim, Kind, name}` and `text` for wrapping.
- `ui::tui::action`, `ui::tui::theme::Accent`.
- `ui::command` for the declaration shapes; the table and every command live under `command/`.
- `super::desk::{Hire, Reply}` for what it asks the desk and what the desk hands back.

## Who calls it

`MainZone` only: `arbitrate` → `request_height` / `assign`, `dispatch` → `deliver_action`, `event` →
`deliver_event`, `render` → `render_rows`, the bus → `hear` / `take_shouts`.

## What it does not know

The history zone, the session, the socket, the database, the agent loop, the picker, and the tree.

## Inside

```text
InputZone
 ├── editor   ── text, cursor, undo, paste, separators
 ├── history  ── the 20-entry submission ring
 ├── statusline ── table + decos
 ├── data     ── model, name, directory, context, money, busy, attachment, hint
 └── pending_exits / pending_hire / shouts
```

- `deliver_action` applies the intent, then re-reads `hang`; a word in range hires completion. A
  command answers with one `Done`: an intent, an exit, a hire, a picked profile, a hint, or `Hold`.
- `deliver_event` decodes only the frames it named: `call meta`, `call config`, `call usage`,
  `stream change`.
- `hear` takes `cwd`, `usage`, `busy`, `prefill`; the statusline hears the rest.

## Must stay in sync

1. `CLAIMS` and the arms of `semantics::translate`.
2. `Data` fields and the decos that read them.
3. Every command file's `META` and the dispatch arm in `command/table.rs`.
4. `ExitRequest` and `Main`'s exit conversion.
5. `Hire` and `Reply` with the desk's hand table.

## Not wired

1. `call usage` is asked once at attach, not after each round; the cost deco can lag.
2. A model switch re-asks the window but keeps the old `used` until the next round closes.
3. The `git` deco scans on its own thread; the interval is a constant.
