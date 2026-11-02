# main — the chat zone

History above, the input box below, the reserved strip between them.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `history` / `input` / `reserved` | the three sub-zones, private |
| `keys: Borrowed` | runtime key loans, checked before the static table |
| `size: Option<TermSize>` | the last size handed down |
| `heights: Heights` | the last row division |

### The zone surface

- `keys()` — the union of the three sub-zones' own claims, in seat order.
- `kinds()` — `Stream`, `Call`, `Notice`: what the sub-zones consume between them.
- `start(size, memo)` — remember the size, read `tree.focus` out of the memo, and ask for the
  structural window and the first load.
- `session(id)` — hand the lifecycle to the three sub-zones; leaving clears the loans.
- `event(frame)` — keep the size when a resize arrives, then hand the frame to every sub-zone
  that declared its class.
- `action(action)` — resolve it to a seat, dispatch, then turn the input zone's exits into
  requests or one handover.
- `render()` — history rows, one gap row, input rows, the reserved strip.

### Rows

`arbitrate` divides the terminal from the bottom: input asks (capped at a fifth), the reserved
strip freezes its height, one gap row, history takes the rest. `settle` re-arbitrates after the
reserved desk has had a chance to change, and broadcasts only when the numbers moved.

### Exits

| exit | becomes |
|---|---|
| `Submit(text)` | `subscribe / submit` |
| `Interrupt` | `post / interrupt` |
| `OpenTree` | `Outcome::Show { to: Tree }` |
| `OpenResume` | `Outcome::Show { to: Resume }` |
| `Quit` | `Outcome::Quit` |
| `New` | reset the transcript and the editor, `Request::Detach` |

## 2. Design intent

- **One host, private children.** The three sub-zones do not know each other.
- **Bottom-up rows.** The input knows its content; the reserved desk freezes; the transcript
  takes what is left.
- **Claims from the consumer.** The key table is assembled, not hand-maintained twice.
- **Exits are data.** A sub-zone says what it cannot finish; `Main` is the only place that turns
  it into a request or a handover.

## 3. Unresolved

1. The three sub-zones still implement the previous surface; `Main` is written against the
   current one, so this zone is wired at its own level only.
2. The static key table is resolved at run time; duplicate claims are not rejected at compile
   time.
3. Terminal-too-small behaviour is clamping, not a designed degradation.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.renderMargin` | history focus margin and overscan (not wired yet) |
| `config.tui.todoFold` | history task-list folding (not wired yet) |
| `config.tui.usage.align` | reserved usage panel placement (not wired yet) |
