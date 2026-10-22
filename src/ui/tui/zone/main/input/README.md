# input — the editor at the bottom

The input sub-zone is the text box and the statusline above it. It turns keys into edits or
intents, routes a submitted line to a message or a command, and declares the exits it cannot
finish itself.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `editor: Editor` | text, cursor, undo stack, paste store |
| `history: History` | the in-memory submission ring, 20 entries |
| `scroll: usize` | first wrapped row shown |
| `rows: u16` | the height the main zone allocated |
| `term: TermSize` | the size used for wrapping between deliveries |
| `finished: bool` | an empty box, so ↑↓ browses history |
| `hang: bool` | the cursor is in a completion trigger word |
| `pending_hire: Option<Hire>` | the hand this zone wants |
| `pending_exits: Vec<ExitRequest>` | submit, interrupt, tree, resume, quit, new |
| `esc_armed` | the first un-lent Esc; a second within the window opens the tree |
| `enter_armed` | the first Enter mid-round; a second within the window submits |
| `cursor_col`, `cursor_row` | the cursor inside the container, recomputed on render |
| `statusline: StatusLine` | the deco table and the row engine |

### Claims

`CLAIMS` is `[AnyChar, Paste]`. Everything else is claimed by `History` or borrowed by
`Reserved`; an unclaimed key is dropped by `Main`.

### Editor

The editor owns the text as a `Vec<char>` plus a cursor index. It exposes insert, delete, word
movement, line and document movement, undo/redo, paste insertion, and `replace_range` for a hand's
reply. `text()` returns the display text with paste markers; `expanded_text()` returns the text
with the paste store expanded and is what leaves on submit.

A paste longer than the collapse thresholds becomes one marker. The marker is replaced on submit;
the submission history keeps the folded form.

Undo records snapshots and coalesces edits inside a window. A cursor move breaks coalescing.

### Input history

`History` is a ring of 20 submitted texts. It is process-global for the zone: `detach` and `/new`
keep it; process exit drops it. `↑` browses it when the box is empty or the history is already
being browsed; `↓` moves forward. Any edit leaves browsing.

### Routing

`route::classify` uses the shared command table:

- a known command becomes `Command(meta, args)`;
- anything else becomes `Message`.

Local commands:

| command | result |
|---|---|
| `/q` | `ExitRequest::Quit` |
| `/resume` | `ExitRequest::OpenResume` |
| `/new` | `ExitRequest::New` |

`/usage` hires the usage panel and also submits the line. Other commands and ordinary messages
become `ExitRequest::Submit`.

### Submit, interrupt, and Esc

- `Enter` submits when the box is non-empty and no round is running.
- While a round is running and the box is non-empty, the first `Enter` interrupts; a second
  `Enter` inside `ESC_TREE_WINDOW` submits.
- `Esc` interrupts. A second un-lent `Esc` inside the window asks for the tree.
- `Alt+Enter` and `Shift+Enter` insert a newline; `Ctrl+J` is the newline fallback.

### Completion hire

After every edit the zone recomputes `hang`: the cursor is in a command word at the start of the
text, or in a path-like word after a space. A fresh hang sets `pending_hire = Completion`. `Main`
takes it, hires the reserved hand, and feeds it the editor's text and cursor.

A hand's `Reply::Replace` is applied with `replace_range`; the draft is not overwritten by a
rewind prefill if it is non-empty.

### Height and rendering

`request_height` wraps the text at the inner width (terminal width minus four border columns),
adds one row, and caps the result at one fifth of the terminal, at least two rows and never more
than the available height.

`render_rows` wraps again, locates the cursor, adjusts the scroll with minimal movement, renders
the statusline, then draws the container: top edge, body rows, bottom edge. It reports the
cursor's column for the reserved zone and the cursor's container-relative row for `Main`.

### Statusline

One row, exactly the terminal width. Each deco is a table entry: id, side (`Left`, `Flex`,
`Right`), order, omission priority, color policy, and a constructor. The current row is
`activity`, `model`, `cwd`, `git`, `cost`, `usage`, `session`. The engine inserts connectors,
allocates the flexible group, and drops low-priority decos when the width is short.

### Lifecycle

`detach` clears exits, hire, hang, armed timers, the editor, and scroll; submission history stays.
`take_requests` drains only `Submit` and `Interrupt`; screen exits remain for `Main`.

### Invariants

1. The input zone never receives a key a hand has borrowed.
2. A hand's height is constant between arrival and departure.
3. An idle desk is one blank row.
4. The statusline is exactly the container's width.
5. A replacement covers the word the cursor was in.
6. The cursor is reported in container coordinates; `Main` owns terminal coordinates.

## 2. Design intent

- **A pure input device.** The editor does not know whether a panel or a popup exists; the
  reserved zone owns that knowledge.
- **One key, one meaning.** The semantics table is context-free; state-dependent behavior lives in
  the editor or in the armed timers, not in a second key map.
- **Exits are data.** The zone declares what it cannot finish; `Main` decides the destination.
- **Submission history is process-scoped.** It is not part of a session and survives detach.
- **The statusline is a table.** Adding or removing a decoration is one entry, not a plumbing
  exercise.
- **Completion is a hand.** The input zone only signals a trigger; the reserved zone decides what
  the candidate list is and how it is drawn.

## 3. Unresolved

1. `Ctrl+C` currently clears the draft; its final meaning is undecided.
2. The git scan interval and the UI language have no config keys; `Setup` uses a three-second
   default and the statusline draws its fixed labels.
3. Completion's model and profile pools come from `view.pools`; the initial fetch is not wired.
4. The first frame after a `Resume` start has no size until an event or action arrives.
5. Terminal-too-small behavior is a clamped container, not a designed degradation.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme` | statusline, editor, and border colors |
| `config.tui.usage.align` | not read here; the reserved usage panel owns it |

The code names a future git-scan interval and a future UI language, but neither key exists in
`TuiConfig` today; `Setup` uses a three-second default and the statusline draws fixed labels.
