# app

The front end's centre: the registry, the display ledger, and the one post box between the zones and the session.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `session: Session` | the only handle; nothing below this file holds one |
| `zones: Vec<Box<dyn Zone>>` | Main, Resume, Tree, Hub, in `ZoneId` order |
| `owner: Option<ZoneId>` | who shows; `None` after the last handover and the loop end |
| `memo: Memo` | what one zone leaves for the next |
| `size: TermSize` | the last size handed down |
| `attached: Option<i64>` | the session the zones were last told about |
| `told: Vec<Option<Option<i64>>>` | per zone: `None` = never told, `Some(id or None)` = told |
| `shared: Shared` | the post box, the one color, and the draft |
| `copy: Vec<String>` | text the loop still owes the terminal's clipboard |
| `asking: VecDeque<Option<ZoneId>>` | who wrote each letter still owed an answer, in the order they left; `None` = this file |
| `finished: bool` | nobody shows |

### Boot

1. Build the shared surface from the startup parameters, and hand each zone its own handle, stamped with that zone's id.
2. If the startup names a session, attach it: `session.attach`, watch, broadcast `session(Some(id))` to the zone that shows.
3. Start the first zone: `start(size, &memo)`, land its asks.

### Delivery

| input | rule |
|---|---|
| frame | the showing zone, and only if its `kinds()` contains the frame's class |
| `local` | the showing zone, always |
| action | the showing zone, and only if its `keys()` contains the claim |

`Local` is the only ask a zone makes of the app itself: `Attach`, `Detach`, `Quit`, `Copy`.

### Mailing

A zone posts a letter into the box it was handed; the box stamps the poster. After every delivery the app drains the box in posting order, hands each letter to the session, and remembers the poster for the answer that follows.

`pump` takes what the connection produced. A reply goes to the zone that wrote that letter, and the poster is forgotten with it; a push (`stream`) or a `local` frame goes to the zone that shows, if it declared that kind. The order is exact because the server answers one request at a time: one letter out, one answer back.

Two frames leave without entering the queue, because the server does not answer them: the watch sent on attach and detach, and the sign-off. The one letter the app writes that *is* answered — the `post mailbox` clear on detach — takes a place in the queue and its answer is dropped.

A `post init` opens a row; its answer is what `created` reads, and what binds the session.

### Handover

`settle(Outcome::Show)` writes the memo the old zone left, makes the new zone the owner, and starts it. `Outcome::Quit` empties the owner and marks the app finished.

## 2. Design intent

- **One handle.** The session lives here and nowhere else.
- **One box.** A zone never names a door to the app; it writes the letter itself, and the app only moves it.
- **The poster is known.** The box stamps every letter, so a reply needs no number and no second table.
- **Declaration, not inspection.** What a zone hears is what it declared.
- **The memo is a transaction.** It exists between one handover and the next, and attach/detach clears it.
- **The draft is a memo too.** What a session will open with is one shared structure, written by whoever the user asked, read when the row is created.

## 3. Unresolved

None.

## 4. Config fields

None. The palette is picked at startup and re-picked on a session's rename; the accent it holds is shared by every zone.
