# input — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the declarations and what the zone exports | private |
| `zone.rs` | `InputZone`, `Default`, `SubZone`, `Pane`, the render entry | private |
| `claim.rs` | `CLAIMS` | private |
| `exit.rs` | `ExitRequest` | private |
| `intent.rs` | one intent applied: editor, history ring, the epilogue | private |
| `submit.rs` | Enter, the command run, writes and their notices, the mailbox accounting | private |
| `esc.rs` | Esc and its double-tap window | private |
| `frame.rs` | the frames this zone decodes | private |
| `boot.rs` | startup questions, session enter/leave, the draft reset, the config keys | private |
| `notice.rs` | the sentences a command earns, both languages | private |
| `ctx.rs` | `Data`, `Ctx`, `Money`, `Context`, `Currency` — what the parts read | private |
| `editor/` | the text, undo, paste store, separators and script classes, submission ring | private |
| `semantics.rs` | key → intent, one context-free table | private |
| `route.rs` | one line classified as a message or a command | private |
| `command/` | the table, the answer shapes, one file per command, the completion trigger | private |
| `render.rs` | the container: top edge, body rows, bottom edge, cursor | private |
| `statusline/` | the row engine, `StatusTheme`, and the seven decos | private |

## What it exposes

Through `SubZone`: `claims`, `kinds`, `deliver_event`, `deliver_action`, `attach`, `detach`,
`take_shouts`, `hear`, plus `super::pane::Pane`'s `request_height` and `assign`.

To `Main`: `take_hire`, `take_exits`, `draft`, `apply_reply`, `clear`, `boot`, `render_rows`,
`cursor_row`, `cursor_col`, `editor`, `statusline`.

The module root re-exports `InputZone`, `ExitRequest`, `Notice`, `Ctx`, `Context`, `Currency` and
`Data`; `command` stays public for the completion hand.

## What it calls

- `ui::tui::zone::{Frame, Request, TermSize, Claim, Kind, name}` and `text` for wrapping.
- `ui::tui::action`, `ui::tui::theme::Accent`.
- `ui::command` for the declaration shapes; the table and every command live under `command/`.
- `ui::lang` for the language and the en/zh pair.
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
 ├── data     ── model, name, directory, context, money, busy
 ├── session  ── the id it follows, or none for a draft
 └── pending_exits / pending_notice / pending_hire / shouts / outbox
```

- `deliver_action` applies the intent, then re-reads `hang`; a word in range hires completion. A
  command answers with one `Done`: a write and its notice, an exit, a hire, a picked profile, a
  refusal notice, or `Hold`.
- `hang` is `command::wants(before, word)`: the same pure rule the completion engine lists by, so
  the hire and the candidates never disagree.
- A write's notice waits in `pending_notice` under its door name; `frame.rs` pops it when that door
  answers, and speaks only when the answer carries no error.
- `deliver_event` decodes only the frames it named: `call meta`, `call config`, `call usage`,
  `post cwd`, `post name`, `post switch`, `post default_model`, `subscribe compact`,
  `subscribe submit`, and the app's `local` answers to a draft's writes. It declares no `stream`
  and no `notice`: the conversation's live frames are not its business.
- `hear` takes `cwd`, `usage`, `tick`, `busy`, `prefill`, and `signal` — the slot taken and the
  fields another front end changed — and the statusline hears the rest.

## Must stay in sync

1. `CLAIMS` and the arms of `semantics::translate`.
2. `Data` fields and the decos that read them.
3. Every command file's `META` and the dispatch arm in `command/table.rs`.
4. `ExitRequest` and `Main`'s exit conversion.
5. `Hire` and `Reply` with the desk's hand table.
6. `submit::door_of` and the receipt names `frame.rs` pops, including the app's local answers.

## Not wired

1. A model switch re-asks the entry; the gauge keeps its count and takes the new window.
2. The `git` deco scans on the frame that finds it due: one `git status` plus one `git branch`,
   tens of milliseconds, once per interval.
