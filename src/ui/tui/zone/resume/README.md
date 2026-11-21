# resume — the session picker

A full-screen list of stored sessions. Every row a picker draws comes from one answer:
`call sessions`. It draws inside the shared page chassis (`zone::page`), the same frame the tree
uses.

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
| `opening: Vec<String>` | the server's opening words, one line each |
| `lang` | the display language; `en` until the config answers |

`Row` carries `id`, `name`, `started_at`, `cwd`, the first user message, the session's size, the
time of its last block, and how many memos it holds. The title is the name, else the first
message, else a label synthesized from the timestamp and the message. The age counts from the
last block, not from `started_at`; a session with memos says how many.

### Header, filter and hints

The head line is `sessions · <count> · <scope>`, where the scope is `this directory` or
`every directory`. Under it come the filter box and one short line saying what it matches; the key
hints wait at the very bottom, under the list.

### Filter box

The filter has its own box: a rule above it, the prompt and the reader's words inside, a rule
below it, and the caption under that. It matches a session's name, first message and directory.
The box, the body and the hints are inset from the frame's sides by a margin the chassis computes,
so the content never sits against the border.

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
| `Backspace`, printable characters, `Ctrl+U`, paste | edit the filter |
| wheel | move the selection |

### Requests

`start` and `Tab` ask for a list; `Enter` on an armed delete sends `post delete` and refetches;
`Enter` on a row attaches it. `start` also asks for `config.ui.lang` and the timezone.

### Rows

One session is three or four physical rows (title, optional preview, meta, gap). The window is
computed in physical rows, so a short list never pushes the last entry off screen.

## 2. Design intent

- **Refetch on entry.** The list is a fact about the outside; last time's snapshot may be stale.
- **Filter keeps indices.** The visible list is a projection; the rows are never reordered.
- **A delete is armed, then confirmed.** One keypress cannot destroy a session.
- **One frame for every page.** The edge, the head, the filter box and the key hints come from
  `zone::page`; the picker supplies its title, its placeholder, its caption, its empty word and its
  hints.

## 3. Unresolved

1. The memo column is a count, not a list: the picker does not ask which memos they are.

## 4. Config fields

| key | what it decides |
|---|---|
| `config.server.platform.timezone` | the fixed offset (`UTC+8`) used to age the list's timestamps |
| `config.ui.lang` | the language of the head, the hints, the empty words and the ages |