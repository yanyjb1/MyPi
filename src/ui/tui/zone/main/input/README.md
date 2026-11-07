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
| `pending_hire` | the hand this zone wants this frame |
| `pending_exits` | submit, interrupt, tree, resume, quit, new |
| `esc_armed` / `enter_armed` | the two double-tap windows |
| `cursor_col`, `cursor_row` | the cursor inside the container, recomputed on render |
| `statusline: StatusLine` | the deco table and the row engine |
| `data: Data` | model, name, directory, context size and window, money, busy, attachment, hint |
| `queue: Queue` | sentences sent while a round was running, head first |
| `in_flight` | the sentence a submit put out, until the door answers |
| `last_mailbox_turn` | the slot-opened turn the queue has accounted for |
| `shouts` | events this zone put out |

### Claims

Every key the editor acts on is named: `AnyChar`, `Paste`, `Enter` (plain, alt, shift), `Esc`, `Tab`,
`Backspace`, `Delete`, the four arrow keys (plain), `Up` with alt, `Left`/`Right`/`Home`/`End` with
ctrl. The history zone is checked first, so `Ctrl+T`, `Ctrl+O` and `Ctrl+↑↓` never reach this table.

### Editor

The editor owns the text as a `Vec<char>` plus a cursor index. It exposes insert, delete, word
movement, line and document movement, undo/redo, paste insertion, and `replace_range` for a hand's
reply. `text()` returns the display text with paste markers; `expanded_text()` returns the text
with the paste store expanded and is what leaves on submit.

A paste longer than the collapse thresholds becomes one marker; the marker is replaced on submit.

### Routing

`route::classify` uses this zone's own table (`command::split`): a known command becomes
`Command(meta, args)`, anything else `Message`. An unknown `/xxx` is a message.

Every command is one file under `command/`, and every file declares its `META` and answers with one
`Done`: an intent (a session action), an exit (`/q`, `/resume`, `/new`), a hire (`/usage`), the
picked profile (`/profile`), a `Hint` line, or `Hold` when an argument is missing or unusable —
`Hold` leaves the line in the box and sends nothing. A plain message leaves as
`ExitRequest::Submit`.

### Submit, interrupt, Esc

- `Enter` submits when the box is non-empty and no round is running.
- While a round is running and the box holds a **message**, the first `Enter` interrupts; a second
  inside the window takes the line as a message. A command skips that step: it executes at once, and
  the server decides whether this connection may write.
- A message taken while a round is running joins the queue and is written to the server's slot; it
  opens nothing now. A reply that opened nothing puts the text back and shouts `retract`.
- `Alt+Up` takes the newest queued sentence back into the box, above what is already there.
- `Esc` interrupts. A second un-lent `Esc` inside the window asks for the tree.
- `Alt+Enter` / `Shift+Enter` insert a newline; `Ctrl+J` is the fallback.

### Completion hire

After every edit the zone recomputes `hang` from the cursor's word. While the word is in range it
sets `pending_hire = Completion`; the main zone seats the desk on the action path and feeds it the
editor's text, cursor and column. A hand's `Reply::Replace` is applied with `replace_range`, and a
reply that lands on a new level seats the next hand at once. Esc is the one thing that keeps a word
closed: the hand remembers the spot until the word changes.

### The pending queue

`queue::Queue` keeps the sentences in order and mirrors the head into the server's one slot: the
head is written when the queue stops being empty, the next head is written when a `mailbox` turn
starts, and clearing the last one clears the slot. `Alt+Up` pops the tail, so the head only moves
when the queue empties.

The only `busy` source that matters is the bus: history shouts `busy` when any round says anything
and `idle` when a round closes; this zone shouts `busy` when a mailbox turn takes the slot and sets
its own flag directly when it sends a message. A round another party opened is known too.

### The data it reads

`call meta` and `stream change` name the model, the session's name and (through the bus) the
directory. A model the zone has not seen makes it ask `call config` for `models.model.<address>`,
which answers the whole entry; the pill takes the name, the gauge the window. `call usage` is the
one answer for money and context, asked at attach, at every round's end and after each request;
the request's cost and the context it carried are the server's to compute, live turn included.
The bus gives `busy` and `prefill`.

### Height and rendering

`pane::Pane`'s `request_height` wraps the text at the inner width, adds one row, and caps the
result at one fifth of the terminal, at least two rows. `render_rows` wraps again, locates the cursor, adjusts the
scroll with minimal movement, ticks the statusline, then draws the container.

### Statusline

One row, exactly the terminal width. The deco table is `activity`, `model`, `cwd`, `git`, `cost`,
`usage`, `session`. Each deco reads the zone's `Data`; none of them can act.

`Data` is filled from three places. The conversation row gives the name and the model address;
the `models.model.<address>` answer gives the model pill and the gauge's denominator; `call usage`
gives the cost in both currencies and where the context stands. The display currency comes from
`config.ui.tui.fx.currency`, never from a model. The gauge's first cell is its marker: `-` for a
measured count, `~` for an estimate (before the first request, and between a compaction and the
request that follows it). `activity` steps one glyph per `tick` shout, so the spinner tracks real
increments. `git` scans on its own frame tick: the first frame at once, then once per configured
interval, and once more whenever a round moved the tree.

### Invariants

1. The input zone never receives a key a hand has borrowed.
2. A hand's height is constant between arrival and departure.
3. An idle desk is one blank row.
4. The statusline is exactly the container's width.
5. The cursor is reported in container coordinates; the main zone owns terminal coordinates.

## 2. Design intent

- **A pure input device.** The editor does not know whether a panel or a popup exists.
- **One key, one meaning.** The semantics table is context-free; state lives in the editor or in
  the armed timers.
- **Exits are data.** The zone declares what it cannot finish; the main zone decides.
- **Submission history is process-scoped.** It survives detach.
- **The statusline is a table.** Adding a decoration is an entry plus a file.
- **Completion is a hand.** The zone only signals a trigger; the desk owns the candidates.

## 3. Unresolved

1. `Ctrl+C` clears the draft; its final meaning is undecided.
2. The usage gauge is hidden until the model's window is known; a count with no window draws no bar.
3. Terminal-too-small behaviour is a clamped container.

## 4. Config fields

| key | use |
|---|---|
| `models.model.<address>` | the whole entry: the model pill's text and the gauge's denominator |
| `config.ui.tui.fx.currency` | the currency the cost pill counts in (`CNY` / `USD`) |
| `config.ui.tui.zone.main.input.deco.gitScanInterval` | seconds between git scans |
| `config.server.api.defaultModel` | the model a draft opens with |
