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

### Busy and idle

`busy` has several callers: the input zone when it puts a message out, the history zone when any
round says anything at all. `idle` has one: the history zone, when a round closes. A zone therefore
reads the last shout and never keeps a second opinion; a front end that guesses idle wrongly writes
to the slot instead of submitting, and the server opens the round anyway.

### Handover

A zone gives the screen up by returning `Outcome`:

| outcome | meaning |
|---|---|
| `Stay` | keep the screen |
| `Show { to, memo }` | name the successor; `memo` is what it leaves behind |
| `Quit` | nobody shows; the loop ends |

The successor is started by the app: it receives the current size and the app's memo, and
decides itself whether to read anything out of it.

### Colors

Two kinds of color, nothing between them:

| kind | where it comes from |
|---|---|
| accent | one color, picked at startup, replaced on rename, carried by a `local / accent` frame |
| fixed | `theme::fixed`, constants taken from the old bundled theme |

The accent is worn by exactly five things: a running tool card's border, the input box's top and
bottom edges, the user card's left bar, the statusline capsule and model name, and the tree's user
node. Everything else is fixed. A tool card's border is accent while running, grey when it
succeeded, red when it failed.

A row carries color slots, not colors (`history::line::Ink`); the palette is resolved when the row
is painted, so a color change costs one repaint and nothing else.

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
3. Streaming animations per tool are not built; a running card is the finished card.

## 4. Config fields

None of this domain's colors come from config: the accent is picked at startup and replaced on
rename. The keys this domain reads are listed per area — `zone/main/history/README.md`,
`zone/main/input/README.md`, `zone/main/reserved/README.md`.
