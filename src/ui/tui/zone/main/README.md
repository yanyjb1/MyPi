# main — the chat zone

History above, the input box below, the desk between them.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `history` / `input` / `reserved` | the three sub-zones, private |
| `keys: Borrowed` | runtime key loans, checked before the static table |
| `size: Option<TermSize>` | the last size handed down |
| `heights: Heights` | the last row division |
| `shouts` | events this zone put out, not yet on the bus |

### The surface

- `keys()` — the union of the three sub-zones' claims, in seat order.
- `kinds()` — all five classes: the zone receives what its children read and forwards it.
- `start(size, memo)` — remember the size, read `tree.focus` out of the memo.
- `session(id)` — attach or detach the three children; on attach ask `call init`, `call meta` and
  the structural window.
- `event(frame)` — keep the size on a resize, read the conversation's directory out of `call meta`
  and `stream change`, hand the frame to every child that declared its class, seat a hand the
  editor asked for, then flush the bus.
- `action(action)` — resolve it to a seat, dispatch, carry the desk's requests, flush.
- `render()` — history rows, one gap row, input rows, the desk's rows, and the cursor's column.

### The bus

One vocabulary: `(name, content)`. A sub-zone puts events in its own `shouts`; the main zone does
the same; `flush` hands each to the other two and repeats until the queue is dry. A shouter never
hears its own event, and a shout raised during delivery lands on the same queue instead of nesting.

Names today: `cwd`, `usage`, `busy`, `prefill`, `queue`, `echo`, `retract`
(`zone::contract::shout::name`).

### Rows

`arbitrate` divides the terminal from the bottom: input asks (capped at a fifth), the desk freezes
its height, one gap row, history takes the rest. `settle` re-arbitrates after the desk had a chance
to change and broadcasts only when the numbers moved.

### Exits

| exit | becomes |
|---|---|
| `Submit(text)` | `subscribe / submit` |
| `Interrupt` | `post / interrupt` |
| `OpenTree` | `Outcome::Show { to: Tree }` |
| `OpenResume` | `Outcome::Show { to: Resume }` |
| `Quit` | `Outcome::Quit` |
| `New` | history and editor cleared, `Request::Detach` |

A `local / disconnected` frame becomes `Request::Quit`: no session, no screen worth keeping.

## 2. Design intent

- **One host, private children.** The three sub-zones never name each other; only the main zone
  holds more than one of them.
- **Events, not channels.** Children do not call each other; they shout a name on the bus and hear
  the names they care about. The main zone is the only party that knows who is on the bus.
- **Bottom-up rows.** The input knows its content, the desk freezes its height, the transcript
  takes what is left.
- **Claims from the consumer.** The key table is assembled from the children's own declarations.
- **Exits are data.** A child says what it cannot finish; the main zone turns it into a request or
  a handover.

## 3. Unresolved

2. The usage gauge's denominator costs one `call config` per model; a missing answer leaves the
   gauge hidden.
3. A rewind's prefill is guessed: history shouts the newest user block it still owns.
4. Terminal-too-small behaviour is clamping, not a designed degradation.

## 4. Config fields

None here. The app asks the theme keys; the history zone asks its own five through `call config`
and the input zone asks the context window.
