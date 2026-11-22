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
| `pending_notice` | one notice per `/model` or `/compact`, waiting for its receipt |
| `esc_armed` / `enter_armed` | the two double-tap windows |
| `cursor_col`, `cursor_row` | the cursor inside the container, recomputed on render |
| `statusline: StatusLine` | the deco table and the row engine |
| `session: Option<i64>` | the conversation this zone follows; `None` = a draft |
| `data: Data` | the followed id's row: model, rungs, effort, name, directory, context, money, busy; `DRAFT` for a draft |
| `queue: Queue` | sentences sent while a round was running, head first |
| `in_flight` | the sentence a submit put out, until the door answers |
| `prepare_pending` | this session's first message, or the first after a compaction, still owes the preparation |
| `struct_seen`, `last_compact` | the structural window has been read, and its newest compaction round |
| `opening_sent` | the line that opens a draft is out, so its `init` structure is not a second primer |
| `shouts` | events this zone put out |

### Claims

Every key the editor acts on is named: `AnyChar`, `Paste`, `Enter` (plain, alt, shift), `Esc`, `Tab`
(plain and shift), `Backspace` (plain, ctrl, alt), `Delete`, the four arrow keys (plain), `Up` with
alt, `Left`/`Right`/`Home`/`End` with ctrl. The history zone is checked first, so `Ctrl+T`, `Ctrl+O`
and `Ctrl+↑↓` never reach this table. A plain `Tab` belongs to the completion hand while a popup is
open; `Shift+Tab` is the editor's own and steps the thinking effort.

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

Every command is one module under `command/`, and every file declares its `META` and answers with
one `Done`: a write plus the notice it owes, an exit (`/q`, `/resume`, `/new`), a hire (`/usage`),
the picked profile (`/profile`), a message with the context it built (`/prepare`), a refusal
notice, or `Hold` when an argument is missing — `Hold` leaves the line in the box and sends
nothing. A plain message is written into a `subscribe submit` letter where it is typed.

`/prepare` builds the workspace preparation: the directory, one terms file (`AGENTS.md`, else
`CLAUDE.md`, else `AI.md`), the files an `AI.md` lists under a `prepare` fence, the manifest's four
sections. It travels in the message's `prefix`, so the history shows
only the user's own words. `config.ui.tui.zone.main.input.autoPrepare` (default off) puts the same
preparation on a session's first message and on the first message after a compaction; the
structural window decides both.

A draft's commands write the shared draft, and the row this zone shows follows it at once: what the
statusline displays is what the next `post init` would carry. That draft rides that letter, so the
name, the model, the effort and the directory it holds are the ones the session opens with.

cannot be entered. Anything else is refused and said. `/compact` needs a session and is refused in a draft.

The directory a leading `~` names is the process's one anchor: the zone asks
`config.server.platform.home` at boot, keeps the answer in its `Data`, and both `/cdp` and the path
completion expand `~` against it. The server answers that key resolved, so an unset key still lands
on this platform's own home.

`/effort` and `Shift+Tab` both walk the rungs the model declares: a session writes the rung it lands
on, a model that declares no rung is refused here, and a rung the model never declared is refused
before the request leaves. `Shift+Tab` in a draft writes the draft, so the rung it lands on is the
one the session opens with; the key is inert on a model that declares none.

Deletion has no command: the resume picker owns it.

### Submit, interrupt, Esc

- `Enter` with a non-empty box and no round running: the line leaves as a turn.
- `Enter` with a non-empty box while a round runs, and the line is a **message**: it joins the queue
  and its head is written to the server's one slot. Nothing opens now.
- A second `Enter` inside 600 ms, the box now empty, interrupts the round; the sentence waiting in the slot opens the next one. Any other key spends the arm.
- A line that opened a round arms that window too, so a twice-tapped `Enter` on a fresh line both opens and stops.
- A command skips all of that: it runs at once, and the server decides whether this connection may
  write.
- `Alt+Up` takes the newest queued sentence back into the box, above what is already there.
- `Esc` interrupts. A second un-lent `Esc` inside 400 ms asks for the tree.
- `Alt+Enter` / `Shift+Enter` insert a newline; `Ctrl+J` is the fallback.

### Notices

A write of the session's own metadata (`/switch`, `/effort`, `/cdp`, `/name`) lands the moment it
leaves: this zone already knows what it asked for, so only a refusal comes back to be said. Its
notice speaks at once, in the display language.

`/model` and `/compact` wait for their receipt (`default_model`, `compact`), and speak only when the
answer carries no error. A refused `post meta` speaks the server's own line, rendered from its key
in the display language.

A refusal decided here (no session for `/usage` or `/compact`, `/effort` on a model that declares
no rung, a profile asked for after the session started, `/memo` without a usable
operation) speaks at once. The line leaves the box either way.

A draft's `/cdp` refusal names the directory it could not land on (`noDir`, a warn). A session that
had to stand in the temp directory instead of the directory its draft carried says so as it opens.

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

Every figure belongs to one id, and `Data::key` names it: a draft's row sits under `DRAFT`, an id
the server never mints. Attaching a session turns the row over to that session, so what a previous
one left never stands in for the new one.

`call meta` names the session's model, name and directory, and the zone asks for it itself whenever
it starts following a row. A model the zone has not seen makes it ask `call config` for
`models.model.<address>`, which answers the whole entry; the pill takes the name, the gauge the
window. A model or a name another front end changed reaches this zone as a `signal` on the bus. In a
draft the boot answer and the one taken on leaving a session name the model, and `/model` moves the
pill from the address it wrote. `call usage` is the one answer for money and context, asked at
attach, at every round's end and after each request.

### Height and rendering

`pane::Pane`'s `request_height` wraps the text at the inner width, adds one row, and caps the
result at one fifth of the terminal, at least two rows. `render_rows` reuses that wrap, locates the
cursor, adjusts the scroll with minimal movement, then draws the container. The statusline's own
clock runs in `tick_statusline`, once a round.

### Statusline

One row, exactly the terminal width. The deco table is `activity`, `model`, `cwd`, `git`, `cost`,
`usage`, `session`. Each deco reads the zone's `Data`; none of them can act. A narrow row drops
whole decos, lowest priority first: the gauge goes before the name, the price and the directory.

`Data` is one row, and `Data::key` is the id it belongs to. The conversation row gives the name, the
directory and the model address; `models.model.<address>` gives the model pill, the gauge's
denominator and the rungs the model declares, so the pill's marker reads the rung on a model that
takes one and `M` on one that declares none; the living thinking effort comes from `call init` and
from each `/effort` receipt, because no row holds it, and a model switch resets it to `auto`, while
a session's own row leaves it alone; `call usage` gives the cost in `config.ui.tui.fx.currency`,
never from a model. The cost pill reads `¥1.23(+0.45)` while the subagents' pool carries a price —
same currency, same precision, no symbol inside the parentheses — and hides the parentheses when the
pool holds none.

The gauge reads `>--8.3%-----:1M`: `~` prefixes the percentage while the count is an estimate, and the
dashes carry no mark of their own. `activity` steps one glyph per `tick` shout. `git` scans on its
own tick: the first one at once, then once per configured interval, and once more when a round
starts or ends.

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

1. `Ctrl+C` clears the draft; its final meaning is undecided.
2. The usage gauge is hidden until the model's window is known.
3. Terminal-too-small behaviour is a clamped container.

## 4. Config fields

| key | use |
|---|---|
| `models.model.<address>` | the whole entry: the model pill's text and the gauge's denominator |
| `config.ui.tui.fx.currency` | the currency the cost pill counts in (`CNY` / `USD`) |
| `config.ui.tui.zone.main.input.deco.gitScanInterval` | seconds between git scans |
| `config.ui.tui.zone.main.input.autoPrepare` | put the workspace preparation on a session's first message and the first after a compaction |
| `config.server.api.defaultModel` | the model a draft opens with |
| `config.ui.lang` | the language of notices, command details and number formats |
