# contract — the two-level zone boundary

The only vocabulary the app and the three major zones share.

## 1. Design / Schema

| type | meaning |
|---|---|
| `Zone` | what the app knows of a major zone |
| `SubZone` | what `Main` knows of history, input and reserved |
| `Kind` | a frame class a zone may declare |
| `Claim` | one action a zone may declare |
| `Request` | one intent: reads, writes, and the app's own asks; the door name lives app-side |
| `Outcome` | what a delivery made the screen do |
| `ZoneId` | `Main`, `Resume`, `Tree` |
| `TermSize` | columns and rows |
| `Memo` | string-to-string, owned by the app, open to the zone it starts |
| `Shout` | one `(name, content)` event on the chat zone's bus |
| `Frame` | one inbound frame: class, name, content; the app builds it |
| `Turns`, `Round` | the structural window a zone reads; the app builds it |

### `Zone`

| method | meaning |
|---|---|
| `keys` | the actions it consumes |
| `kinds` | the frame classes it handles; `local` is never listed |
| `start` | the app gives it the screen: size and the memo |
| `session` | the conversation was entered (`Some`) or left (`None`) |
| `event` | one frame it declared, or any `local` one |
| `action` | one input it claimed |
| `structure` | the structural window after `Request::Structure` |
| `render` | this zone's rows |
| `cursor` | the hardware cursor, if any |

### `SubZone`

`claims`, `kinds`, `deliver_event(frame, size)`, `deliver_action(action)`, `attach(session,
size)`, `detach`, `borrowed`, `structure`, `take_shouts`, `hear`. `Main` is the only host. Rows
are `Main`'s own business: `main::pane::Pane` takes the ask, the grant and the broadcast.

### `Kind`

`Call`, `Post`, `Subscribe`, `Stream`, `Notice`. `Kind::of(kind, name)` classifies one frame and
lifts `stream / notice` out of `Stream`. `local` returns `None`: it is delivered unconditionally.

### `Claim`

`Key(Key)`, `AnyChar`, `Paste`, `Wheel`, `WheelShift`. `Claim::of(action)` projects one action.

### `Request`

| variant | lands as |
|---|---|
| `Send { kind, name, content, answer }` | one session frame; the app fills in the session |
| `Attach(i64)` | `session.attach` + `subscribe watch` + the broadcast |
| `Detach` | `session.detach` + the broadcast |
| `Turns` | `session.structure()`, translated by the app, handed to the zone that shows |
| `Quit` | the app ends the loop |

### `Shout`

One event a sub-zone (or the main zone) puts out: `name` and `content`. The main zone delivers it
to the other two and repeats until the queue is dry; a shouter never hears its own event, and a
shout raised while delivering lands on the same queue.

The names in use: `cwd` (main), `usage`, `busy`, `prefill` (history and input), `queue`,
`echo`, `retract` (input to history and the desk).

### `Outcome`

`Stay`; `Show { to, memo }` — the successor and what to leave in the memo; `Quit`.

## 2. Design intent

- **Declaration is the interface.** A zone says what it hears and what it claims; the app never
  inspects a payload.
- **Two levels, no skipping.** The app talks to major zones; a major zone talks to its sub-zones.
- **The memo is part of the boundary.** It is owned by the app and handed to the zone being
  started; nothing else can reach it.
- **Requests are data.** A zone never touches the session; it names work and the app lands it.

## 3. Unresolved

1. Two major zones may claim the same input: only one of them shows, and the app gives an action
   to that one, and only when it declared the claim.

## 4. Config fields

None. The contract carries values; it reads no configuration.
