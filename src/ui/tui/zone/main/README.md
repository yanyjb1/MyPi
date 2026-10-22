# main — the chat zone

`Main` is the screen the user lives in: the transcript above, the input box below, and the
reserved strip between them. It hosts three private sub-zones and arbitrates the rows they share.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `history: HistoryZone` | the transcript |
| `input: InputZone` | the editor and its statusline |
| `reserved: ReservedZone` | completion or usage, one hand at a time |
| `keys: Borrowed` | runtime key loans, checked before the static table |
| `size: Option<TermSize>` | the last terminal size |
| `heights: Heights` | the last row division |
| `pending_exits: Vec<ExitRequest>` | exits produced by `Main` itself |
| `pending_requests: Vec<Request>` | requests waiting for the next take |

The sub-zones are private. The rest of the front end reaches them only through `SubZone`.

### Row arbitration

`arbitrate` divides the terminal from the bottom up:

1. `input.request_height(size)` asks for its content, capped at one fifth of the terminal.
2. `reserved.request_height(size)` returns its frozen report, or one idle row.
3. One blank row is inserted between history and the rest when there is room.
4. `history` receives the remainder.

`Heights` records all four values. `settle` re-arbitrates after the reserved desk has had a chance
to drop or take a hand, and broadcasts only when the result changed. The broadcast goes to all
three sub-zones; each decides whether it cares.

### Key routing

`keys::resolve` checks runtime loans first, then the static table. The static table is built from
the sub-zones' own `CLAIMS`, in seat order: `History`, `Input`, `Reserved`.

- A claimed action goes to exactly one sub-zone.
- An unclaimed action is dropped.
- `Input` is also the path through which a hand is hired: after an input action, `Main` takes
  `pending_hire`, hires the hand, feeds it the editor text and cursor, and applies any reply.
- `Reserved` actions also produce replies for the editor.

### Exit requests

`InputZone` declares exits instead of performing them:

| exit | result |
|---|---|
| `Submit(text)` | `Request::Submit` |
| `Interrupt` | `Request::Interrupt` |
| `OpenTree` | `Handover { Tree, requests }` |
| `OpenResume` | `Handover { Resume, requests }` |
| `Quit` | `Handover { None, requests }` |
| `New` | reset history, clear input, `Request::Detach` |

`Main` drains its own `pending_requests`, then the history, reserved, and input queues, then the
exit queues. If an exit changes the screen, the requests travel in the `Handover`. Otherwise they
stay in `pending_requests` for the next take.

### Requests

`MainZone::take_requests` returns, in order: `pending_requests`, history, input, reserved. The app
takes this list once per frame.

### Rendering

`render`:

1. `settle` the reserved desk and re-arbitrate.
2. Draw history rows.
3. Insert `heights.gap` blank rows.
4. Draw input rows.
5. Report the input cursor column to the reserved zone.
6. Draw reserved rows.

`cursor` converts the input's container-relative cursor into terminal coordinates and clamps it
outside the reserved strip.

### Lifecycle

`attach` and `detach` are broadcast to all three sub-zones. `detach` also clears `pending_exits`,
`pending_requests`, and runtime loans. `take_over` sets the size and reads the `tree.focus` memo
alias; a value `turn:block` is passed to `HistoryZone::focus` and then deleted.

## 2. Design intent

- **One host, private children.** The three sub-zones do not know each other; `Main` is the only
  place that sees all three.
- **Bottom-up rows.** The input knows its content; the reserved desk freezes its height; the
  transcript takes what is left. This keeps the cursor and the panel stable while the transcript
  grows.
- **Claims from the consumer.** The table is assembled, not hand-maintained in a second place.
- **Runtime loans beat static claims.** A hand that borrows keys is the current owner of those
  keys; the static table resumes when the loan ends.
- **Exits are data.** A sub-zone declares what it cannot finish; `Main` is the only place that
  turns an exit into a handover.
- **One request list per frame.** The app has no queue; `Main` aggregates and hands over.

## 3. Unresolved

1. The static table is resolved at run time; duplicate claims are not rejected at compile time.
2. The frame loop that calls `deliver_*`, `render`, and `take_requests` does not exist.
3. Terminal-too-small behaviour is clamping, not a designed degradation.
4. `Ctrl+C` has no decided meaning; the input zone currently maps it to clearing the draft.
5. `HistoryZone::CLAIMS` is a public constant used by the table; the other sub-zones expose their
   claims through `SubZone`, so the table is a mix of const and method access.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme` | palette for history and input |
| `config.tui.renderMargin` | history focus margin and overscan |
| `config.tui.todoFold` | history task-list folding |
| `config.tui.usage.align` | reserved usage panel placement |
