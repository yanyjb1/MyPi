# contract — the zone boundary

The only vocabulary the app and the three major zones share.

## 1. Design / Schema

| type | meaning |
|---|---|
| `Zone` | what the app knows of a major zone |
| `Kind` | a frame kind a zone may declare |
| `Claim` | one action a zone may declare |
| `Mail` | the post box every zone drops its letters in |
| `Local` | one ask that never leaves this process |
| `Shared` | the post box, the one color and the draft, handed out at boot |
| `Draft` | what a session will open with |
| `Outcome` | what a delivery made the screen do |
| `ZoneId` | `Main`, `Resume`, `Tree` |
| `TermSize` | columns and rows |
| `Memo` | string-to-string, owned by the app, open to the zone it starts |
| `Shout` | one `(name, content)` event on the chat zone's bus |
| `Frame` | one frame either way: kind, name, payload; the mail box sends it up, the session hands it down |
| `Structure`, `Turn` | the structural window a zone reads |

### `Zone`

| method | meaning |
|---|---|
| `bind` | the shared surface arrives once, at boot |
| `keys` | the actions it consumes |
| `kinds` | the frame kinds it handles; `local` is never listed |
| `start` | the app gives it the screen: size and the memo |
| `session` | the conversation was entered (`Some`) or left (`None`) |
| `event` | one frame it declared, or any `local` one |
| `action` | one input it claimed |
| `tick` | one frame's side effects, before any drawing |
| `revision` | bumped whenever the zone's rows would differ |
| `anim` | how often time alone rewrites the rows; `None` = never |
| `render` | this zone's rows; it changes nothing |
| `cursor` | the hardware cursor, if any |

### `Frame`

Three fields: `kind`, `name`, `payload`. A zone writes one and posts it; the app hands it to the session, and the answer comes back on the same `kind` and `name`. `payload` is the writer's business and nobody else reads it.

### `Mail`

One box, held by every zone. `Mail::at(id)` is the handle one zone posts with: every frame it sends names it as the poster, so an answer needs no second table.

### `Kind`

`Call`, `Post`, `Subscribe`, `Stream`, `Notice`. `Kind::of(kind, name)` names the kind of one frame and lifts `stream / notice` out of `Stream`. `local` returns `None`: it is delivered unconditionally.

### `Claim`

`Key(Key)`, `AnyChar`, `Paste`, `Wheel`, `WheelShift`. `Claim::of(action)` projects one action.

### `Local`

`Attach(i64)`, `Detach`, `Quit`, `Copy(String)`. The first three move the screen; the last one is the clipboard.

### `Draft`

The directory, name, model, profile and effort a row will open with, and whether the user picked the directory. Whoever the user asked writes it; whoever opens the row reads it into `post init`.

### `Shout`

One event a sub-zone (or the main zone) puts out: `name` and `content`. The main zone delivers it to the other two and repeats until the queue is dry; a shouter never hears its own event, and a shout raised while delivering lands on the same queue.

### `Outcome`

`Stay`; `Show { to, memo }` — the successor and what to leave in the memo; `Quit`.

## 2. Design intent

- **Declaration is the interface.** A zone says what it hears and what it claims; the app never inspects a payload.
- **The zone knows the door.** A letter is written by the party that wants it, so no table between the zone and the wire has to be kept in step with anything.
- **Two levels, no skipping.** The app talks to major zones; a major zone talks to its sub-zones.
- **The memo is part of the boundary.** It is owned by the app and handed to the zone being started; nothing else can reach it.
- **One color.** The palette is a handle every zone holds; a rename repaints the whole front end at once.

## 3. Unresolved

1. Two major zones may claim the same input: only one of them shows, and the app gives an action to that one, and only when it declared the claim.

## 4. Config fields

None. The contract carries values; it reads no configuration.
