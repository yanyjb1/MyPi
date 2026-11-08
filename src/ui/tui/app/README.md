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
| `told: Vec<Option<i64>>` | the session each zone has last been told about |
| `cwd`, `model` | what a new session is created with |
| `waiting` | a message sent before the session row existed |
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
app keeps the seven it owns — `Submit`, `Copy`, `Attach`, `Detach`, `Structure`, `Quit`,
`PickProfile`.
A `Submit` with no session attached first creates the row (`post init` with `cwd`, `model` and the
picked profile), parks the text, and sends it after the id lands. `Structure` asks the session for
the structural window and hands one `Turns` snapshot to the zone that shows; a nested `Structure`
request is dropped.

### Lifecycle

- `attach(id)`: follow, watch, clear the memo, tell the zone that shows, land.
- `detach()`: stop following, clear the memo, tell the zone that shows, land.
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

## 3. Unresolved

1. One waiting message at a time: a second submit before the row lands replaces the first.
2. The draft carries no name or profile; both are sent as `null`.

## 4. Config fields

None. The palette is fixed; the accent is picked at startup (`Accent::pick()`) and re-picked on a
rename.
