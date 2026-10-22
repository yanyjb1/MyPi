# tui — the terminal front end

The terminal front end of mypi. It draws a screen, reads a keyboard, and asks `ui::session` for
the data behind both. It knows nothing about the database, the model, the socket, or the agent
loop.

## 1. Design / Schema

### Three layers

`App` owns the screen. It holds three zones and the ledger that says which one shows:

| zone | screen |
|---|---|
| `Main` | the chat: transcript, input box, reserved strip |
| `Resume` | the session picker |
| `Tree` | the block tree of the current conversation |

Only `Main` has sub-zones: `history`, `input`, `reserved`. They are private to `Main`. At most
one zone shows at a time; showing is the right to interact with the session.

### Two input lines

`Action` is user behaviour: a key, a paste, a wheel notch. It goes to exactly one seat.

`Event` is everything else: a round started or ended, blocks arrived, a tool started or finished,
a notice, an idle tick, a deadline, the connection dropped or returned, the terminal resized,
suspended, resumed, focused, lost, gone, or broken. It is broadcast to the showing zone.

Neither line carries data. A consumer reads what it needs from `Ctx`: the borrowed session view,
the terminal size, and the handover memo when that window is open.

### Claims and the key table

A sub-zone declares the input it consumes as `Claim` values: one concrete key, any printable
character, a paste, a wheel notch, or a shifted wheel notch. `Main` builds one table from those
declarations and resolves an `Action` to a `Seat`. Runtime loans are checked before the static
table; an unclaimed action is dropped.

### Handover

A showing zone gives the screen up by returning `Handover { to, requests }`. The old zone puts
its pending requests into `requests`; the app registers the new zone and hands it the first
context. Naming nobody (`to: None`) ends the front end. There is no second path for changing the
screen.

### Requests

A zone never touches the socket. It declares work for the loop: submit, interrupt, rewind, a
history ask, a session list, attach, delete, detach, or a tree range. Screen changes are not
requests; they are handovers.

### Lifecycle

`App::attach` is data first, display second: a different session detaches the old one, clears the
memo, records the new id, broadcasts `attach`, and warms the transcript. `App::detach` broadcasts
`detach` and clears the session and memo. Every zone hears both, attached or not.

### Identity and memo

An `App` has a `Uuid`. The same credential is meant to accompany every session call; it never
travels through a zone.

`Memo` is a string-to-string slot table with `Write`, `Read`, and `Delete`. It is open only during
`hand_over` and `take_over`. Zones use it by alias; the app stores content without interpreting
it. It is cleared on attach and detach.

### Invariants

1. An event carries no data.
2. A zone never touches the socket.
3. At most one zone shows.
4. One input goes to one seat.
5. Nobody showing means the loop exits.
6. A handover is the only way the screen changes.
7. The memo is open only during handover and takeover.

## 2. Design intent

- **Two input lines.** User behaviour has one consumer; world events may have several. One line
  for both would need a router that decides precedence, which this design refuses.
- **Events carry no data.** A signal says that something happened; the details already live in
  the session view. A per-event snapshot would duplicate state and let the copies drift.
- **Claims come from the consumer.** Whoever reads a key names it, so no other zone can mislabel
  it. `Main` only relays.
- **Showing is one id.** Showing is the right to talk to the session; the server cannot tell which
  zone is talking and does not need to.
- **The size rides the context.** A zone that has just taken over still has to lay out. The first
  context carries the size, and every delivery carries the current one.
- **Handover carries the queue.** A zone that loses the screen cannot keep work for a loop that
  will not ask it again. The queue travels with the handover.
- **The memo is a transaction window.** Cross-zone facts are written and read only while one zone
  gives the screen to another; outside that window there is no shared mutable state.
- **The session view is the only data source.** Zones read the same borrowed view; they do not
  keep a second copy of session state.

## 3. Unresolved

1. The frame loop does not exist: nothing produces events, nothing draws, and nothing lands
   requests.
2. `lib.rs` does not declare `ui`, and `Cargo.toml` has no `ratatui` or `crossterm`: the tree is
   not in the build.
3. The session API lacks `Call::Init`, `Reply::Init`, `Session::tree_range`, and
   `Session::register` / `unregister`. History's `Init`, Tree's range, and App identity therefore
   have no landing site.
4. `Ctrl+C` has no decided meaning.
5. Terminal-too-small behaviour has no design; the current arithmetic clamps instead of degrading.
6. The static key table is resolved at run time; duplicate claims are not rejected at compile
   time.
7. History's seam is preserved as an anchor, but the layout's `seam` argument does not select a
   cut point yet.
8. Tree currently projects one fork; the multi-fork geometry exists but is not fed by session
   structure.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme.accent` | accent color override |
| `config.tui.theme.gold` | money and warning color override |
| `config.tui.theme.black` | capsule background override |
| `config.tui.theme.muted` | secondary gray override |
| `config.tui.renderMargin` | blocks kept around the sight line, and the history overscan |
| `config.tui.todoFold` | task-list folding policy |
| `config.tui.usage.align` | horizontal placement of the usage panel |
| `config.tui.fx.usdCny` | USD-to-CNY rate used when the usage panel folds currencies |
