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
| `theme: ThemeConfig`, `palette_seen` | the palette being assembled from `call config` answers |
| `told: Vec<Option<i64>>` | the session each zone has last been told about |
| `cwd`, `model` | what a new session is created with |
| `waiting` | a message sent before the session row existed |
| `finished: bool` | nobody shows |

### Boot

1. Ask the five palette keys (see `THEME_KEYS` in `app.rs`) through `call config`.
2. If the startup names a session, attach it: `session.attach`, `subscribe watch`, broadcast
   `session(Some(id))` to the zone that shows, land what it asks for.
3. Start the first zone: `start(size, &memo)`, land its requests.

The memo carries the launch directory (`cwd`) for the picker; `attach` and `detach` reset the
memo but always write it back.

### Delivery

| input | rule |
|---|---|
| frame | the showing zone, and only if its `kinds()` contains the frame's class |
| `local` | the showing zone, always |
| action | the showing zone, and only if its `keys()` contains the claim |

Every delivery returns requests; they are landed at once, in order.

### Landing

`Request::Send` gets the attached session filled in (unless the zone named one) and goes to the
session. A `subscribe / submit` with no session attached first creates the row (`post init` with
`cwd` and `model`), parks the message, and sends it after the id lands. `Attach` / `Detach` are
the lifecycle. `Structure` asks the session for the structural window and hands one `Turns`
snapshot to the zone that shows; a nested `Structure` request is dropped.

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
- **No payload knowledge.** `kind` picks the route; `content` is never read except for the five
  palette answers, which are this layer's own configuration.
- **Declaration, not inspection.** What a zone hears is what it declared.
- **The memo is a transaction.** It exists between one handover and the next, and attach/detach
  clears it.

## 3. Unresolved

1. A second palette ask (after the first) is unanswered; the palette is fixed at boot.
2. One waiting message at a time: a second submit before the row lands replaces the first.
3. The draft carries no name or profile; both are sent as `null`.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.theme.name` | the JSON theme to load |
| `config.ui.tui.theme.accent` / `.gold` / `.black` / `.muted` | colour overrides |
