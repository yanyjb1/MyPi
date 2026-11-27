# input — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

1. `mod.rs` — the declarations and what the zone exports; private.
2. `zone.rs` — `InputZone`, `Default`, the entry points `Main` calls, `Pane`, the render entry; private.
3. `claim.rs` — `CLAIMS`; private.
4. `exit.rs` — `ExitRequest`; private.
5. `intent.rs` — one intent applied: editor, history ring, the epilogue; private.
6. `submit.rs` — Enter, the command run, writes and their notices, the draft's settle, the mailbox accounting; private.
7. `frame.rs` — the frames this zone decodes; private.
8. `boot.rs` — startup questions, the draft reset, the config keys; private.
9. `draft.rs` — the memo a session opens from, shared with every zone until its row exists; private.
10. `queue.rs` — sentences sent while a round ran, and the server's one slot they mirror; private.
11. `notice.rs` — the sentences a command earns, as i18n keys; private.
12. `ctx.rs` — `Data` (one row per id, `DRAFT` for a draft), `Ctx`, `Money` (with the subagents' pool), `Context`, `Currency`, which is what the parts read; private.
13. `editor/` — the text, undo, paste store, separators and script classes, submission ring; private.
14. `semantics.rs` — key → intent, one context-free table; private.
15. `route.rs` — one line classified as a message or a command; private.
16. `command/` — the table, the answer shapes, one module per command, and the completion trigger, while `prepare/` also holds the payload; private.
17. `render.rs` — the container: top edge, body rows, bottom edge, cursor; private.
18. `statusline/` — the row engine, `StatusTheme`, and the decos with their table; private.

## What it exposes

To `Main` as calls on the zone itself: `kinds`, `deliver_event`, `deliver_action`, `attach`, `detach`, `take_shouts`, `hear`, plus `super::pane::Pane`'s `request_height` and `assign`.

To `Main`: `take_hire`, `take_exits`, `draft`, `apply_reply`, `clear`, `boot`, `bind`, `render_rows`, `cursor_row`, `cursor_col`, `editor`, `statusline`.

The module root re-exports `InputZone`, `ExitRequest`, `Notice`, `Ctx` and `Currency`; `Data` stays inside the zone, and `command` stays public for the completion hand.

## What it calls

- `ui::tui::zone::{Frame, Local, Shared, TermSize, Claim, Kind, name}` and `text` for wrapping.
- `ui::tui::action`, `ui::tui::theme::Accent`.
- `ui::tui::lang` for the display language; command details and notices render through `crate::i18n`.
- `serde_yaml` and `std::fs` for the preparation's declared paths, terms and manifest.
- `super::desk::{Hire, Reply}` for what it asks the desk and what the desk hands back.

## Who calls it

`MainZone` only: `arbitrate` → `request_height` / `assign`, a claimed key → `deliver_action`, `event` → `deliver_event`, `render` → `render_rows`, the bus → `hear` / `take_shouts`.

## What it does not know

The history zone, the session, the socket, the database, the agent loop, the picker, and the tree.

## Inside

```text
InputZone
 ├── editor   ── text, cursor, undo, paste, separators
 ├── history  ── the 20-entry submission ring
 ├── statusline ── table + decos
 ├── data     ── the followed id's row: model, name, directory, context, money, busy
 ├── session  ── the id it follows, or none for a draft
 └── pending_exits / pending_notice / pending_hire / shouts / outbox
```

- `attach` turns `data` over to the row's id, then asks `call meta` and `call usage` for it; no other zone asks on its behalf, and `detach` turns the row back to `DRAFT`.
- `deliver_action` applies the intent, then re-reads `hang`; a word in range hires completion. A command answers with one `Done`: a write and its notice, an exit, a hire, a picked profile, a refusal notice, or `Hold`.
- `hang` is `command::wants(before, word)`: the same pure rule the completion engine lists by, so the hire and the candidates never disagree.
- A metadata write lands the moment it leaves; only `/model` and `/compact` wait in `pending_notice` for their door's answer, and speak only when it carries no error.
- `deliver_event` decodes only the frames it named: `call init`, `call meta`, `call config`, `call usage`, `post meta`, `post default_model`, `subscribe compact`, `subscribe submit`, and the app's `local` answers to a draft's writes. It declares no `stream` and no `notice`: the conversation's live frames are not its business.
- `hear` takes `cwd`, `usage`, `tick`, `busy`, `prefill`, and `signal` — the slot taken and the fields another front end changed — and the statusline hears the rest.
- The thinking effort is memory-only and belongs to the row: a draft's copy rides `post init`, a session's comes from `call init` and every `/effort` receipt, and an unset one reads `auto` in the pill, since a draft never receives `call init`.

## Must stay in sync

1. `CLAIMS` and the arms of `semantics::translate`.
2. `Data` fields and the decos that read them.
3. Every command file's `META` and the `run` it points to.
4. `ExitRequest` and `Main`'s exit conversion.
5. `Hire` and `Reply` with the desk's hand table.
6. The receipt names `frame.rs` pops and the `name` each command writes on its letter.

## Not wired

1. A model switch re-asks the entry; the gauge keeps its count and takes the new window.
2. The `git` deco scans on the frame that finds it due: one `git status` plus one `git branch`, tens of milliseconds, once per interval.