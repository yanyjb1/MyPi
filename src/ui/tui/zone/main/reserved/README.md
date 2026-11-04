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
| `pools: Option<Pools>` | the model and profile names |
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
- `place(cwd)` and `pools(pools)` — what the desk knows and hands over;
- `text(text, cursor)` and `col(col)`;
- `on_key(key) -> Option<Reply>`;
- `draw(width, rows, palette) -> Vec<Line>`;
- `left()`.

`Reply` is `Replace { from, to, text }`; indices are char indices. The input zone applies it.

`IDLE_ROWS` is one blank row, so the layout does not jump when a hand arrives or leaves.

### Frames

- On attach the desk asks `call pools` once and keeps the answer; the completion hand gets it.
- Hiring the usage panel asks `call usage`; the panel decodes `groups` and keeps only what it draws.
- The desk hears `cwd` from the bus and hands it to the hand on duty.
- The desk hears `queue` (`{count}`) and keeps the number. Idle, that number is the one row it
  draws: `N messages queued, alt+up to edit.`; with no hand on duty and nothing queued, the row is
  blank. A hand on duty owns the row and hides the count.

### Lifecycle

- `hire(who)` calls `left()` on a different current hand, records the new hire, clears the report,
  calls `arrived`, and feeds the hand the workspace and pools it already knows.
- `settle()` drops a hand whose `stays()` is false and takes the report of a fresh hand once
  `ready()` is true.
- `pane::Pane`'s `request_height` returns the frozen report's rows, or `IDLE_ROWS`.
- `assign` records the allocated rows, at least `IDLE_ROWS`.
- `borrowed` returns the report's keys.
- `deliver_action` passes a key to the hand and stores any reply.
- `detach` calls `left()`, then clears the seat, report, reply, rows, workspace, and pools.

## 2. Design intent

- **One seat.** A second panel would have to fight the first for the row and the keys; there is
  exactly one desk.
- **A frozen height.** The strip never grows or shrinks while a hand is sitting: the input and the
  transcript would both move under the reader.
- **A hand is a part of the input.** It borrows the editor's keys and draws its own rows; it never
  talks to the session itself, it asks the desk.
- **Data flows down.** The desk owns the workspace and the pools; a hand is handed them, never
  reads them from a global.

## 3. Unresolved

1. `/usage`'s horizontal alignment has no reader: the panel draws the full width.
2. The completion popup's anchor column is pushed after the first draw; the first frame of a hire
   draws at column zero.
3. A failed `call pools` leaves completion without model or profile names.

## 4. Config fields

| key | use |
|---|---|
None: the palette is fixed and the accent comes from the app.
