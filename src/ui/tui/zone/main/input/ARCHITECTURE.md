# input — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `InputZone`, `CLAIMS`, `ExitRequest`, the frame answers it decodes | private |
| `ctx.rs` | `Ctx`, `Data`, `Money`, `Context`, `Currency` — what the parts read | private |
| `editor/` | the text, undo, paste store, separators, submission ring | private |
| `semantics.rs` | key → intent, one context-free table | private |
| `route.rs` | one line classified as a message or a command | private |
| `command/` | the table (`table.rs`), the answer shapes (`call.rs`), one file per command | private |
| `render.rs` | the container: top edge, body rows, bottom edge, cursor | private |
| `statusline/` | the row engine, `StatusTheme`, and the eight decos | private |

## What it exposes

Through `SubZone`: `claims`, `kinds`, `deliver_event`, `deliver_action`, `attach`, `detach`,
`take_shouts`, `hear`, plus `super::pane::Pane`'s `request_height` and `assign`.

To `Main`: `take_hire`, `take_exits`, `draft`, `apply_reply`, `clear`, `render_rows`,
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
- `hang` is `command::wants(before, word)`: the same pure rule the completion engine lists by, so
  the hire and the candidates never disagree.
- `deliver_event` decodes only the frames it named: `call meta`, `call config`, `call usage`,
  `post cwd`, `post name`, `post switch`, `stream change`. `/cdp` shows what the receipt names; a
  `/name` writes the name it asked for once the receipt comes back.
- `hear` takes `cwd`, `usage`, `tick`, `busy`, `prefill`; the statusline hears the rest. A `usage`
  shout means a request landed: the zone answers it with one `call usage`, the only cost source.

## Must stay in sync

1. `CLAIMS` and the arms of `semantics::translate`.
2. `Data` fields and the decos that read them.
3. Every command file's `META` and the dispatch arm in `command/table.rs`.
4. `ExitRequest` and `Main`'s exit conversion.
5. `Hire` and `Reply` with the desk's hand table.

## Not wired

1. A model switch re-asks the entry; the gauge keeps its count and takes the new window.
2. The `git` deco scans on the frame that finds it due: one `git status` plus one `git branch`,
   tens of milliseconds, once per interval.
