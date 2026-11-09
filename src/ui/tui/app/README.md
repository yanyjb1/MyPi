# app

The front end's centre: the registry, the display ledger, and the only exit to the session.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `session: Session` | the only handle; nothing below this file holds one |
| `zones: Vec<Box<dyn Zone>>` | Main, Resume, Tree, in `ZoneId` order |
| `owner: Option<ZoneId>` | who shows; `None` after the last handover and the loop end |
| `memo: Memo` | what one zone leaves for the next |
| `size: TermSize` | the last size handed down |
| `attached: Option<i64>` | the session the zones were last told about |
| `told: Vec<Option<Option<i64>>>` | per zone: `None` = never told, `Some(id or None)` = told |
| `cwd`, `model` | what a new session is created with |
| `profile`, `name` | the draft's picks; they travel with the row |
| `opening` | a message sent before the session row existed |
| `copy: Vec<String>` | text the loop still owes the terminal's clipboard |
| `finished: bool` | nobody shows |

### Boot

1. If the startup names a session, attach it: `session.attach`, `subscribe watch`, broadcast
   `session(Some(id))` to the zone that shows, land what it asks for.
2. Start the first zone: `start(size, &memo)`, land its requests.

The memo carries the launch directory (`cwd`) for the picker; `attach` and `detach` reset the
memo but always write it back.

### Delivery

| input | rule |
|---|---|
| frame | the showing zone, and only if its `kinds()` contains the frame's class |
| `local` | the showing zone, always |
| action | the showing zone, and only if its `keys()` contains the claim |

Every delivery returns requests; they are landed at once, in order.

`Request::Copy` is the one ask the app keeps for somebody else: it joins `copy`, and the frame loop
takes it with `take_copy` and writes the terminal's clipboard between two draws.

### Landing

A `Request` is an intent: the app names the door. `wire::land` carries every session action; the
app keeps the ones that are its own — `Submit`, `SetCwd`, `Rename`, `SwitchModel` in a draft,
`Copy`, `Attach`, `Detach`, `Structure`, `Quit`, `PickProfile`.
A `Submit` with no session attached first creates the row (`post init` with `cwd`, `name`, `model`
and the picked profile), parks the text, and sends it after the id lands. A draft's `SetCwd`,
`Rename` and `SwitchModel` land here and answer with a `local` frame named after the same door, so
the input zone can speak its receipt; a session's land on the server instead.
`Structure` asks the session for the structural window and hands one `Turns` snapshot to the zone
that shows; a nested `Structure` request is dropped.

### Lifecycle

- `attach(id)`: follow, watch, clear the memo and the draft's picks, tell the zone that shows, land.
- `detach()`: stop following, clear the memo and the draft's picks, tell the zone that shows, land.
- `tell()` compares one zone's last told value with `attached`; `None` means never told, so a detach
  reaches the zones exactly as an attach does.
- `pump()` reconciles: if the session attached somewhere else (a delete landed upstream), the app
  detaches; a session the app attached itself is already consistent.

### Handover

`settle(Outcome::Show)` writes the memo the old zone left, makes the new zone the owner, and
starts it. `Outcome::Quit` empties the owner and marks the app finished.

## 2. Design intent

- **One handle.** The session lives here and nowhere else; zones produce requests, the app lands
  them.
- **No payload knowledge.** `kind` picks the route; `content` is read only for the terminal size,
  a create's reply, and the rename that re-picks the accent.
- **Declaration, not inspection.** What a zone hears is what it declared.
- **The memo is a transaction.** It exists between one handover and the next, and attach/detach
  clears it.
- **A draft answers like the server.** What a draft lands itself is answered with a `local` frame
  named after the same door, so a zone has one receipt rule.

## 3. Unresolved

1. One waiting message at a time: a second submit before the row lands replaces the first.

## 4. Config fields

None. The palette is fixed; the accent is picked at startup (`Accent::pick()`) and re-picked on a
rename.
