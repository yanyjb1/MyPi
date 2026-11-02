# tui

The terminal front end: ratatui + crossterm on top of `ui::session`.

## 1. Design / Schema

### Two lines and one horn

| line | carries | goes to |
|---|---|---|
| action | a key, a paste, a wheel notch | the zone that claimed it |
| frame | `kind`, `name`, `content` | the showing zone, if it declared the kind |
| local | the app's own facts (resize, focus, session loss) | the showing zone, unconditional |
| shout | `name`, `content` | every other zone of the chat area |

A shout is the chat zone's own bus: a sub-zone puts one out, the main zone hands it to the other
two, and nobody hears its own. A zone that does not know a name does nothing. The shouter is
always the zone, never one of its parts.

Only the showing zone hears anything; the other two major zones are inert until the app starts
one of them.

### Declarations

A major zone declares:

- `keys()` — every action it consumes, as `Claim` values. One action goes to exactly one zone.
- `kinds()` — the frame classes it handles: `Call`, `Post`, `Subscribe`, `Stream`, `Notice`.
  `local` is never declared.

`notice` is the one class the app lifts out of `stream` by name, so a zone can hear notices
without hearing every text delta.

### Size

The app hands the size to the zone it starts, and again in `local / resized` when the terminal
changes. Nothing else carries a size; a zone keeps the last one it was given.

### Handover

A zone gives the screen up by returning `Outcome`:

| outcome | meaning |
|---|---|
| `Stay` | keep the screen |
| `Show { to, memo }` | name the successor; `memo` is what it leaves behind |
| `Quit` | nobody shows; the loop ends |

The successor is started by the app: it receives the current size and the app's memo, and
decides itself whether to read anything out of it.

### The loop

```text
resize?  -> local / resized
pump     -> session.forward() -> deliver
draw     -> the showing zone's rows and cursor
poll     -> one action or one focus change
```

A failed draw ends the loop; nothing tries to draw again.

## 2. Design intent

- **Declare, do not guess.** A zone says what it hears; the app never inspects a payload.
- **Only the showing zone talks.** The display right is the right to interact; a zone that is not
  showing neither hears nor asks.
- **`local` is the floor.** Terminal facts and connection facts arrive without a declaration,
  because every zone has to survive them.
- **The app is a postman with a ledger.** It classifies by `kind`, fills in the session id, and
  lands requests; it holds no conversation state of its own.

## 3. Unresolved

1. Suspend and resume have no trigger path: raw mode turns Ctrl+Z into an ordinary key, so
   nothing suspends the front end.
2. A zone started again re-asks what it needs; there is no warm handover of data.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme.name`, `.accent`, `.gold`, `.black`, `.muted` | the palette; asked by the app at boot |
