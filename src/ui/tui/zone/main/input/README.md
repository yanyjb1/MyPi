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
| `data: Data` | model, name, directory, context size and window, money, busy |
| `shouts` | events this zone put out |

### Claims

Every key the editor acts on is named: `AnyChar`, `Paste`, `Enter` (plain, alt, shift), `Esc`, `Tab`,
`Backspace`, `Delete`, the four arrow keys (plain), `Left`/`Right`/`Home`/`End` with ctrl. The
history zone is checked first, so `Ctrl+T`, `Ctrl+O` and `Ctrl+↑↓` never reach this table.

### Editor

The editor owns the text as a `Vec<char>` plus a cursor index. It exposes insert, delete, word
movement, line and document movement, undo/redo, paste insertion, and `replace_range` for a hand's
reply. `text()` returns the display text with paste markers; `expanded_text()` returns the text
with the paste store expanded and is what leaves on submit.

A paste longer than the collapse thresholds becomes one marker; the marker is replaced on submit.

### Routing

`route::classify` uses the shared command table: a known command becomes `Command(meta, args)`,
anything else `Message`.

Local commands: `/q` → quit, `/resume` → the picker, `/new` → reset. `/usage` hires the panel and
submits the line. Everything else becomes `ExitRequest::Submit`.

### Submit, interrupt, Esc

- `Enter` submits when the box is non-empty and no round is running.
- While a round is running and the box is non-empty, the first `Enter` interrupts; a second inside
  the window submits.
- `Esc` interrupts. A second un-lent `Esc` inside the window asks for the tree.
- `Alt+Enter` / `Shift+Enter` insert a newline; `Ctrl+J` is the fallback.

### Completion hire

After every edit the zone recomputes `hang`. A fresh hang sets `pending_hire = Completion`; the main
zone seats the desk and feeds it the editor's text, cursor and column. A hand's `Reply::Replace` is
applied with `replace_range`.

### The data it reads

`call meta` and `stream change` name the model, the session's name and (through the bus) the
directory. A model the zone has not seen makes it ask `call config` for
`models.model.<address>.contextWindow`; the answer is the usage gauge's denominator. `call usage`
gives the money. The bus gives `used`, `busy` and `prefill`.

### Height and rendering

`request_height` wraps the text at the inner width, adds one row, and caps the result at one fifth
of the terminal, at least two rows. `render_rows` wraps again, locates the cursor, adjusts the
scroll with minimal movement, renders the statusline, then draws the container.

### Statusline

One row, exactly the terminal width. The deco table is `activity`, `model`, `cwd`, `git`, `cost`,
`usage`, `session`. Each deco reads the zone's `Data`; none of them can act.

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
2. The git scan interval has no config key; `Setup` defaults to three seconds.
3. The usage gauge is hidden until both its numbers arrive.
4. Terminal-too-small behaviour is a clamped container.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.theme` | statusline, editor, and border colors |
| `models.model.<address>.contextWindow` | the gauge's denominator |
