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
| `pending_notice` | one notice per write, waiting for that write's receipt |
| `esc_armed` / `enter_armed` | the two double-tap windows |
| `cursor_col`, `cursor_row` | the cursor inside the container, recomputed on render |
| `statusline: StatusLine` | the deco table and the row engine |
| `session: Option<i64>` | the conversation this zone follows; `None` = a draft |
| `data: Data` | model, name, directory, context size and window, money, busy |
| `pending_name` | the name a `/name` asked for, until its receipt |
| `queue: Queue` | sentences sent while a round was running, head first |
| `in_flight` | the sentence a submit put out, until the door answers |
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

A word ends at punctuation or whitespace, or where the script family changes: Latin, Han, Kana,
Hangul, Greek and Cyrillic are distinct families, and `.`, `_`, `-` join whatever sits beside them.
`Ctrl+←/→`, `Ctrl+W` and `Alt+D` all read that one table.

A paste longer than the collapse thresholds becomes one marker; the marker is replaced on submit.

### Routing

`route::classify` uses this zone's own table (`command::split`): a known command becomes
`Command(meta, args)`, anything else `Message`. An unknown `/xxx` is a message.

Every command is one file under `command/`, and every file declares its `META` and answers with one
`Done`: a write plus the notice it owes, an exit (`/q`, `/resume`, `/new`), a hire (`/usage`), the
picked profile (`/profile`), a refusal notice, or `Hold` when an argument is missing — `Hold` leaves
the line in the box and sends nothing. A plain message leaves as `ExitRequest::Submit`.

`/profile` is refused here once a session id exists. `/name`, `/cdp` and `/switch` work in a draft:
the app keeps them until the row opens. A draft's `/cdp` lands only on a directory that exists — `~`
expanded, a relative path read from where the draft stands — because the name travels into `init`,
and a directory that is not there cannot be entered. Anything else is refused and said.
`/compact` needs a session and is refused in a draft.
Deletion has no command: the resume picker owns it.

### Submit, interrupt, Esc

- `Enter` with a non-empty box and no round running: the line leaves as a turn.
- `Enter` with a non-empty box while a round runs, and the line is a **message**: it joins the queue
  and its head is written to the server's one slot. Nothing opens now.
- A second `Enter` inside 600 ms, the box now empty, interrupts the round; the sentence waiting in
  the slot opens the next one. Any other key spends the arm.
- A command skips all of that: it runs at once, and the server decides whether this connection may
  write.
- `Alt+Up` takes the newest queued sentence back into the box, above what is already there.
- `Esc` interrupts. A second un-lent `Esc` inside 300 ms asks for the tree.
- `Alt+Enter` / `Shift+Enter` insert a newline; `Ctrl+J` is the fallback.

### Notices

A write (`/switch`, `/model`, `/cdp`, `/name`, `/compact`) owes one line. The text is built from what
this zone sent, in the display language, and spoken only when the receipt for that door arrives:
`cwd`, `name`, `switch`, `default_model`, `compact`. A receipt carrying an error speaks nothing —
except the one the app writes itself for a draft's `/cdp`, whose answer names the directory it could
not land on (`noDir`, a warn).
Draft writes are answered by the app with the same door names, prefixed `local`.

A refusal decided here (no session for `/usage` or `/compact`, a profile asked for after the
session started, `/memo` without a usable operation) speaks at once. The line leaves the box
either way.

Notices travel as a shout; the history zone draws them as short rows. A `/compact` receipt also
puts `compact start` on the bus: a compaction round is running, and any zone may draw it.

### Completion hire

After every edit the zone recomputes `hang` from the cursor's word. While the word is in range it
sets `pending_hire = Completion`; the main zone seats the desk on the action path and feeds it the
editor's text, cursor and column. A hand's `Reply::Replace` is applied with `replace_range`, and a
reply that lands on a new level seats the next hand at once. Esc is the one thing that keeps a word
closed: the hand remembers the spot until the word changes.

### The pending queue

`queue::Queue` keeps the sentences in order and mirrors the head into the server's one slot: the
head is written when the queue stops being empty, the next head is written when the server says it
took the slot, and clearing the last one clears the slot. `Alt+Up` pops the tail, so the head only
moves when the queue empties.

The only `busy` source that matters is the bus: history shouts `busy` when any round says anything
and `idle` when a round closes; this zone also shouts `busy` when it sends a message, so the first
Enter after a send already knows a round is coming.

### The data it reads

`call meta` names the session's model, name and directory. A model the zone has not seen makes it
ask `call config` for `models.model.<address>`, which answers the whole entry; the pill takes the
name, the gauge the window. A model or a name another front end changed reaches this zone as a
`signal` on the bus. In a draft the boot answer and the one taken on leaving a session name the
model; `/model` moves the pill itself, from the address it wrote. `call usage` is the one answer for
money and context, asked at attach, at every round's end and after each request.

### Height and rendering

`pane::Pane`'s `request_height` wraps the text at the inner width, adds one row, and caps the
result at one fifth of the terminal, at least two rows. `render_rows` reuses that wrap, locates the
cursor, adjusts the scroll with minimal movement, then draws the container. The statusline's own
clock runs in `tick_statusline`, once a round.

### Statusline

One row, exactly the terminal width. The deco table is `activity`, `model`, `cwd`, `git`, `cost`,
`usage`, `session`. Each deco reads the zone's `Data`; none of them can act.

`Data` is filled from three places. The conversation row gives the name and the model address;
the `models.model.<address>` answer gives the model pill and the gauge's denominator; `call usage`
gives the cost in both currencies and where the context stands. The display currency comes from
`config.ui.tui.fx.currency`, never from a model. The gauge reads `~` in front of the percentage
when the count is an estimate; the dashes carry no mark. `activity` steps one glyph per `tick`
shout. `git` scans on its own tick: the first one at once, then once per configured interval, and
once more when a round starts or ends.

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
- **A receipt is a `done`.** The zone already knows what it sent, so the line it speaks is built
  from its own request; the receipt only says when. A display that changes on a write waits for
  the same receipt.
- **Status is state, notices are events.** Refusals and write confirmations belong in the
  transcript, not in a row of standing facts.
- **Submission history is process-scoped.** It survives detach.
- **The statusline is a table.** Adding a decoration is an entry plus a file.
- **Completion is a hand.** The zone only signals a trigger; the desk owns the candidates.

## 3. Unresolved

1. A server refusal (an error receipt) speaks nothing; the server's own notice is the only trace.
2. `Ctrl+C` clears the draft; its final meaning is undecided.
3. The usage gauge is hidden until the model's window is known.
4. Terminal-too-small behaviour is a clamped container.

## 4. Config fields

| key | use |
|---|---|
| `models.model.<address>` | the whole entry: the model pill's text and the gauge's denominator |
| `config.ui.tui.fx.currency` | the currency the cost pill counts in (`CNY` / `USD`) |
| `config.ui.tui.zone.main.input.deco.gitScanInterval` | seconds between git scans |
| `config.server.api.defaultModel` | the model a draft opens with |
| `config.ui.lang` | the language of notices, command details and number formats |
