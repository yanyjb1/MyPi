# contract — the two-level zone boundary

The contract is the only vocabulary shared by the app, the three major zones, and the three
sub-zones of `Main`.

## 1. Design / Schema

### `Ctx`

Every delivery carries:

| field | meaning |
|---|---|
| `view: &View` | the borrowed session read model |
| `size: TermSize` | terminal columns and rows |
| `memo: Option<&Memo>` | the handover memo, open only during handover and takeover |

### `Zone`

A major zone implements:

| method | meaning |
|---|---|
| `deliver_event` | receive a broadcast event |
| `deliver_action` | receive one claimed action; return a handover to give the screen up |
| `attach` / `detach` | hear a session lifecycle change |
| `hand_over` | lose the screen; the memo is open |
| `take_over` | take the screen; the memo is open |
| `render` | return this zone's rows |
| `cursor` | return the hardware cursor, if any |
| `take_requests` | return session work produced since the last take |

### `SubZone`

A sub-zone implements:

| method | meaning |
|---|---|
| `claims` | the static input claims it owns |
| `deliver_event` / `deliver_action` | receive a broadcast or one claimed action |
| `attach` / `detach` | hear a session lifecycle change |
| `borrowed` | keys it borrows at run time |
| `request_height` | rows it needs for a terminal size |
| `assign` | rows the main zone allocated, plus the terminal size |
| `on_height_changed` | the row broadcast; `true` means consumed |
| `take_requests` | session work produced since the last take |

### `Request`

| variant | asks for |
|---|---|
| `Submit(String)` | send one message |
| `Interrupt` | stop the running round |
| `Rewind { turn, block, text }` | cut back to a block; text is for the clipboard |
| `View(Ask)` | a history ask: init, jump, fetch, or settled |
| `List { under }` | read the session list, optionally scoped to a directory |
| `Attach(i64)` | attach to a session |
| `Delete(i64)` | delete a session |
| `Detach` | leave the current session |
| `TreeRange { start, end }` | read raw material for a block-id range |

### `Handover` and `ZoneId`

`Handover { to: Option<ZoneId>, requests: Vec<Request> }`. `ZoneId` is `Main`, `Resume`, or
`Tree`. `to: None` means nobody shows and the loop exits. The losing zone carries its queue in
`requests`.

### `Claim` and `Seat`

`Claim` is `Key(Key)`, `AnyChar`, `Paste`, `Wheel`, or `WheelShift`. `Seat` is `History`, `Input`,
or `Reserved`. The main zone resolves one action to one seat.

### `Ownership`

One `Option<ZoneId>`. `register` replaces it; `showing` reads it. There is no stack and no second
ledger.

## 2. Design intent

- **Two levels, no skipping.** The app talks to major zones; `Main` talks to sub-zones. A
  sub-zone never reaches the app.
- **One delivery carries everything.** The session view and the size ride the context, so an
  event can stay a signal.
- **The memo is part of the contract, not a global.** It is `Some` only in the two transfer
  methods, which makes the window explicit.
- **Requests are not screen changes.** A zone asks for session work; only a handover changes what
  is shown.
- **Claims belong to the consumer.** The table is assembled from the sub-zones' own declarations.
- **A losing zone carries its queue.** The app does not keep a second pending list.

## 3. Unresolved

1. The loop that lands `Request` does not exist.
2. `TreeRange` has no session-side landing call.
3. Duplicate claims are not rejected at compile time; the static table is resolved at run time.
4. `Ctrl+C` has no decided meaning.
5. Terminal-too-small degradation is not designed; sizes are clamped by the consumers.

## 4. Config fields

None. The contract carries values; it does not read configuration.
