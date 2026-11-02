# resume — the session picker

A full-screen list of stored sessions. Every row a picker draws comes from one answer:
`call sessions`.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `rows: Vec<Row>` | the last answer, decoded |
| `visible: Vec<usize>` | indices that passed the filter |
| `filter` | the search word; matched case-insensitively |
| `selected` / `scroll` | position inside `visible` |
| `scope` | this directory, or every directory |
| `cwd` | the launch directory, read out of the memo key `cwd` when the page starts |
| `loaded` | an answer arrived at least once |
| `pending_delete` | the session a Delete armed |
| `failed` | the last answer refused something |

`Row` carries `id`, `name`, `started_at`, `cwd`, the first user message, the session's size, the
time of its last block, and how many memos it holds. The title is the name, else the first
message, else a label synthesized from the timestamp and the message. The age counts from the
last block, not from `started_at`; a session with memos shows `memo N`.

### Keys

| key | does |
|---|---|
| `↑` / `↓` | move the selection |
| `PageUp` / `PageDown` | move by a page |
| `Home` / `End` | first / last visible row |
| `Enter` | attach, or confirm an armed delete |
| `Delete` | arm a delete |
| `Esc` | back to `Main` |
| `Tab` | toggle the scope, then refetch |
| `Backspace`, printable characters, `Ctrl+U` | edit the filter |
| wheel | move the selection |

### Requests

`start` and `Tab` ask for a list; `Enter` on an armed delete sends `post delete` and refetches;
`Enter` on a row attaches it.

### Rows

One session is three or four physical rows (title, optional preview, meta, gap). The window is
computed in physical rows, so a short list never pushes the last entry off screen.

## 2. Design intent

- **Refetch on entry.** The list is a fact about the outside; last time's snapshot may be stale.
- **Filter keeps indices.** The visible list is a projection; the rows are never reordered.
- **A delete is armed, then confirmed.** One keypress cannot destroy a session.

## 3. Unresolved

1. `failed` is shown as one fixed line; the refusal's own text is not carried.
2. The memo column is a count, not a list: the picker does not ask which memos they are.

## 4. Config fields

None.
