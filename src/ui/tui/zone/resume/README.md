# resume — the session picker

`Resume` is a full-screen list of stored sessions. It filters, scopes, previews, and asks to
attach or delete. It draws itself; it does not own the session.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `size: Option<TermSize>` | the last terminal size |
| `visible: Vec<usize>` | indices into `view.rows` that pass the filter |
| `seen: usize` | the row count at the last refilter |
| `filter: String` | the search text |
| `selected: usize` | position inside `visible` |
| `scroll: usize` | first visible list index, written back by the renderer |
| `scope: Scope` | `Current` or `All` |
| `cwd: PathBuf` | the directory for `Current` |
| `loaded: bool` | whether a list ever arrived |
| `pending_delete: Option<i64>` | the session a Delete armed |
| `pending: Vec<Request>` | list, attach, and delete requests |

### Opening

`begin(cwd)` remembers the directory, clears the filter and the armed delete, resets the scroll,
and queues `Request::List { under }`. `take_over` sets the size and calls `begin` with the view's
working directory.

`attach` does not detach. Selecting a different session emits `Request::Attach(id)` with the
handover; selecting the same session is a no-op in `App::attach`.

### The list

Rows come from `view.rows`, a `&[SessionRow]`. `sync` refilters only when the row count changes;
the filter keeps indices, not copies.

`Scope::Current` scopes to `cwd`; `Scope::All` sends `under: None`. `Tab` toggles the scope and
refetches.

The filter matches, case-insensitively, the session id, its explicit name, the first user message,
and its working directory. `Backspace` edits it; `Ctrl+U` clears it.

### What one row shows

- Title: explicit name, else the first line of the first user message, else the synthesized
  display name.
- Preview: the first user message, but only when the session has an explicit name.
- Meta: age, size, and a shortened working directory.

### Delete

`Delete` arms `pending_delete` for the selected id. `Enter` then confirms: it queues
`Request::Delete(id)` and a refetch, and stays on the page. A refusal is drawn as `- fail`; a
successful delete takes the row with it. Any other key except `Enter`, `Delete`, or `Esc` cancels
the arm.

### Keys

| key | does |
|---|---|
| `↑` / `↓` | one row |
| `PageUp` / `PageDown` | eight rows |
| `Home` / `End` | first / last visible row |
| `Tab` | current directory / all directories |
| printable characters | append to the filter |
| `Backspace` | delete one filter character |
| `Ctrl+U` | clear the filter |
| `Delete` | arm a delete |
| `Enter` | attach, or confirm a delete |
| `Esc` | return to `Main` |
| wheel | one row |

### Rendering

A card-framed page: header with count and scope, search row, notices, list, footer. The list
window is measured in physical rows, so a short list does not push the last card off screen.
Selection is the only position indicator; the selected title uses the accent color.

### Invariants

1. The visible set is a list of indices into `view.rows`; filtering never copies rows.
2. A delete needs two presses: arm, then confirm.
3. `Esc` returns to `Main` and carries the pending requests.
4. Opening the picker does not detach the current session.

## 2. Design intent

- **A picker is a page, not a panel.** It owns the whole screen while it shows.
- **Filter by index.** The session list belongs to the view; the picker only decides what is
  visible.
- **Refetch on open and on scope change.** The list is a fact about the outside and may be stale.
- **Delete is two-step.** A destructive action gets an explicit confirmation on the same row.
- **Attach travels with the handover.** The picker does not attach itself; the app owns the
  session lifecycle.
- **The page leaves no trace.** Detach clears the list, filter, selection, and loaded flag.

## 3. Unresolved

1. `Request::List`, `Request::Attach`, and `Request::Delete` have no landing loop.
2. The first frame of a `Resume` start has no size until an event or action arrives.
3. A refused delete is shown, but the picker does not clear `pending_delete` on every refusal
   path; the page relies on the next action.
4. The list has no per-session keyboard shortcut or preview pane.
5. The filter has no cursor; editing is append-and-backspace only.

## 4. Config fields

None. The picker reads the session view and the terminal size.
