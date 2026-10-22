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
| `last_usage` | the last `(lang, groups)` handed to the panel |

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

- `arrived(ctx)` — before the first report;
- `ready()` — the desk waits for this before freezing the height;
- `stays()` — `false` drops the hand;
- `on_event(ev, ctx)`;
- `text(text, cursor)` and `col(col)`;
- `on_key(key) -> Option<Reply>`;
- `draw(ctx, rows) -> Vec<Line>`;
- `left()`.

`Reply` is `Replace { from, to, text }`; indices are char indices. The input zone applies it.

`IDLE_ROWS` is one blank row, so the layout does not jump when a hand arrives or leaves.

### Lifecycle

- `hire(who, ctx)` calls `left()` on a different current hand, records the new hire, clears the
  report, and calls `arrived`.
- `settle()` runs at frame start and on delivery: it drops a hand whose `stays()` is false, and
  takes the report of a fresh hand once `ready()` is true.
- `request_height` returns the frozen report's rows, or `IDLE_ROWS`.
- `assign` records the allocated rows, at least `IDLE_ROWS`.
- `borrowed` returns the report's keys.
- `deliver_action` passes a key to the hand and stores any reply.
- `detach` calls `left()`, then clears the seat, report, reply, rows, and last usage.
- `take_requests` is empty: the desk has no session work of its own.

### Usage

`sync_usage(view)` reads `view.usage` and compares `(lang, groups)` with the last value. It calls
`show_usage` only when the value changed, so the panel's selected row survives a frame. The panel
reports three header rows plus one row per model, capped at 24 rows. It borrows Esc, ↑, ↓, and Tab;
Esc closes it, Tab switches the number language.

The panel's numbers come from the server; the front end only lays them out. `set_usage_config`
reads `UsageAlign` once at assembly.

### Rendering

Idle: one blank row. On duty: the hand draws its own rows at the allocated height. The desk does
not draw the hand's content itself.

### Invariants

1. One hand at a time.
2. A hand's height is constant from report to departure.
3. An idle desk is one blank row.
4. The desk never claims keys up front; it only borrows through a report.
5. The desk never reaches the session, the store, or the socket.

## 2. Design intent

- **One seat, one roster.** A new hand is added in one match and one roster entry.
- **Power goes down.** The desk owns the seat and the report; a hand only answers questions.
- **A hand is an extension of the input zone.** It cannot reach outside; whatever it needs done
  goes back through the input zone.
- **Frozen height.** A report is taken once, so a popup cannot resize under the reader's cursor.
- **The desk owns the reply slot.** A hand computes; the input zone writes.
- **Usage is fed, not fetched.** The panel is a pure layout of numbers that arrive from outside.

## 3. Unresolved

1. Completion's model and profile pools are not fetched at startup; `view.pools` is the intended
   source.
2. The usage panel's multi-language and absolute/percentage views are a later wave.
3. The usage panel's `UsageAlign` is read, but the panel's width and row cap are fixed in code.
4. The bridge mark and the model context window are not sent by the server yet.
5. The loop that delivers events, actions, and usage updates does not exist.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.usage.align` | which side the usage panel hugs |
| `config.tui.theme` | colors for the panel and the popup |
