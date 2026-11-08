# reserved — the strip above the editor

The reserved sub-zone is a desk for temporary hands. Today the roster has two: the completion
popup and the usage panel. At most one hand is on duty, its height is frozen on arrival, and it
leaves before another sits down.

## 1. Design / Schema

### The desk

| field | meaning |
|---|---|
| `completion: CompletionService` | the completion hand |
| `usage: UsagePanel` | the usage hand |
| `on_duty: Option<Hire>` | who is sitting at the desk |
| `report: Option<Report>` | the frozen report: rows, keys, feeds |
| `want: u16` | rows the main zone allocated |
| `reply: Option<Reply>` | a replacement waiting for the input zone |
| `cwd: String` | the workspace paths complete against |
| `queued: usize` | sentences waiting in the slot, as the input zone counts them |
| `outbox: Vec<Request>` | frames the desk owes the session |

`Hire` is `Completion` or `Usage`. `ReservedZone::CLAIMS` is empty: the desk only borrows keys
through a hand's report.

### The hand contract

A hand reports:

| field | meaning |
|---|---|
| `rows` | rows it needs; fixed from arrival to departure |
| `keys` | keys it borrows while on duty |
| `text` | it wants the editor's text and cursor after every edit |
| `col` | it wants the cursor's column after every draw |

A hand also has:

- `arrived()` — before the first report;
- `ready()` — the desk waits for this before freezing the height;
- `stays()` — `false` drops the hand;
- `on_event(event, content)` — one bus event;
- `place(cwd)` — the workspace, handed over on arrival and on every change;
- `text(text, cursor)` and `col(col)`;
- `on_key(key) -> Option<Reply>`;
- `draw(width, rows, palette) -> Vec<Line>`;
- `left()`.

`Reply` is `Replace { from, to, text }`; indices are char indices. The input zone applies it.

`IDLE_ROWS` is one blank row, so the layout does not jump when a hand arrives or leaves.

### The usage panel

`UsagePanel` draws two summary rows, a blank row, a head row, and one row per model; its height is
`4 + models`, capped at 24. One tier is picked per frame, widest first:

| tier | label | token columns | heads and summary words |
|---|---|---|---|
| room | `provider:modelid` | exact counts | full |
| narrowed | `display(provider)` | percentages | full |
| shortened, English only | `display(provider)` | percentages | `Mo` `To` `In` `Ou` `CR` `CW`; `TT` `CHR` `Rds` |
| columns leave, widest first | as above | cache write, cache read, output, input | as above |
| last | the label is clipped | the total column alone | as above |

Every cell is cut to the width the plan hands it, so no line is wider than the strip. The block keeps
its natural width and `config.ui.tui.zone.main.reserved.usageAlign` places it: `left`, `center` (the
default) or `right`. Tab swaps exact counts for percentages in the room tier only. The hit rate is
`cached / (prompt + completion)` over the whole session.

The four token columns are one partition of that model's own total: `Input` is the prompt no cache
reported, then `CacheRead` and `CacheWrite`, then `Output`. A percentage column is that column over
the same row's total, so one row's four percentages add up to 100 whatever the other rows did.

`ALIGN_KEY` is asked on every usage hire, and the answer reaches the panel through `("call",
"config")`.

### Frames

- Every completion hire asks `call pools`; the answer goes straight to the hand on duty.
- Hiring the usage panel asks `call usage`; the panel decodes `groups` and keeps only what it draws.
- The desk hears `cwd` from the bus and hands it to the hand on duty.
- The desk hears `queue` (`{count}`) and keeps the number. Idle, that number is the one row it
  draws: `N messages queued, alt+up to edit.`; with no hand on duty and nothing queued, the row is
  blank. A hand on duty owns the row and hides the count.

### Lifecycle

- `hire(who)` calls `left()` on a different current hand, records the new hire, clears the report,
  calls `arrived`, asks the hand's pools, and feeds it the workspace.
- `settle()` drops a hand whose `stays()` is false and takes the report of a fresh hand once
  `ready()` is true.
- `pane::Pane`'s `request_height` returns the frozen report's rows, or `IDLE_ROWS`.
- `assign` records the allocated rows, at least `IDLE_ROWS`.
- `borrowed` returns the report's keys.
- `deliver_action` passes a key to the hand and stores any reply.
- `detach` calls `left()`, then clears the seat, report, reply and rows; the workspace stays,
  being where the app runs rather than a row's field.

## 2. Design intent

- **One seat.** A second panel would have to fight the first for the row and the keys; there is
  exactly one desk.
- **A frozen height.** The strip never grows or shrinks while a hand is sitting: the input and the
  transcript would both move under the reader.
- **A hand is a part of the input.** It borrows the editor's keys and draws its own rows; it never
  talks to the session itself, it asks the desk.
- **The word decides.** The input asks for the hand on every edit of a trigger word, so deleting a
  character and typing the level again reopens the same popup; Esc is the only thing that keeps one
  word closed, and it is spent as soon as the word changes.
- **Names are asked on every hire.** The completion hand holds the seat until the answer lands, so
  the popup opens with its height already fixed; a hand that is merely dismissed leaves at once.
- **The height counts the level, not the word.** A completion locks the row count of the level it
  opened on — the whole command table, one directory's entries, the whole name pool — and keeps it
  through every later level of the same completion, so narrowing the word never resizes the strip.
- **Data flows down.** The desk owns the workspace; a hand is handed it, never
  reads them from a global.

## 3. Unresolved

1. The completion popup's anchor column is pushed after the first draw; the first frame of a hire
   draws at column zero.
2. A failed `call pools` leaves completion without model or profile names until the next hire.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.zone.main.reserved.usageAlign` | `left`, `center` or `right`: where the usage panel's block sits. |
