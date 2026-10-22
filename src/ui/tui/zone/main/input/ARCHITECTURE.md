# input — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | `InputZone`, `ExitRequest`, `CLAIMS`, the `SubZone` implementation | private to `Main` |
| `route.rs` | `Route`, `classify` — message or command | private |
| `semantics.rs` | `Intent`, `translate` — key to intent | private |
| `render.rs` | the container: edges, body, cursor mapping | private |
| `editor/mod.rs` | `Editor`, `Effect`, text and cursor operations | private |
| `editor/history.rs` | `History` — the submission ring | private |
| `editor/undo.rs` | `Snapshot`, `UndoStack`, `EditKind` | private |
| `editor/paste.rs` | `PasteStore`, markers, collapse thresholds | private |
| `editor/separators.rs` | `SeparatorTable` — word boundaries | private |
| `statusline/mod.rs` | `StatusLine`, `Entry`, `Deco`, `Setup`, `Side`, `ColorPolicy` | private |
| `statusline/layout.rs` | groups, connectors, omission | private |
| `statusline/theme.rs` | `StatusTheme`, `Token`, `ALL_TOKENS` | private |
| `statusline/deco/` | `ROW` and one file per decoration | private |

## What it exposes

Through `SubZone`:

- `claims`, `deliver_event`, `deliver_action`
- `attach`, `detach`
- `request_height`, `assign`, `on_height_changed`
- `take_requests`

To `Main`:

- `CLAIMS`
- `pending_exits`, `pending_hire`
- `editor`, `history`, `scroll`, `rows`
- `cursor_col`, `cursor_row`
- `clear`, `prefill`, `apply_reply`
- `render_rows`

## What it calls

- `zone::main::geometry` for inner width, container height, scroll, and cursor position.
- `zone::main::reserved::{Hire, Reply}`.
- `zone::{Action, Claim, Ctx, Event, Key, KeyCode, Request, TermSize}`.
- `text::{wrap, Wrapped}`.
- `theme::Palette` and `statusline::StatusTheme`.
- `commands` for classification and local command names.
- `ui::session::View` for the statusline and the busy state.

## Who calls it

`MainZone` only:

- `arbitrate` → `request_height`, `assign`
- `settle` / `broadcast` → `on_height_changed`
- `dispatch` → `deliver_action`
- `deliver_event` → `deliver_event`
- `attach` / `detach`
- `render` → `render_rows`
- `take_requests`

`Main` also reads `pending_hire`, `pending_exits`, `editor.text`, `editor.cursor`, and
`cursor_col` directly; they are the input/reserved handshake.

## What it does not know

The app, the history zone, the reserved zone's internals, the picker, the tree, the socket, the
database, and the agent loop.

## Must stay in sync

1. `semantics::Intent` and `apply`: every intent needs an effect.
2. `ExitRequest` and `Main::dispatch::exits`: every variant needs a destination.
3. `Claim` values and `Main`'s static table.
4. `request_height` and `geometry::container_height`.
5. `render_rows`' cursor report and `Main::cursor_position`.
6. `StatusLine::ROW` and the deco trait: a new deco is a file plus one row.
7. `detach` and the submission history: transient state goes, history stays.

## Not wired

1. `Setup.git_scan` is a three-second default; no config key is read.
2. `view.pools` is not fetched at startup, so completion has no model or profile names.
3. The statusline's language and several `statusLine*` tokens are not authoritative yet.
4. The loop that delivers actions and takes requests does not exist.
