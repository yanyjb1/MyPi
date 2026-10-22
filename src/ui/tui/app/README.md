# app — the screen owner

The app is the top of the terminal front end. It owns the three major zones, decides which one
shows, and is the only place where a handover is executed.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `id: Uuid` | this front end's identity |
| `memo: Memo` | aliases written during handover and read during takeover |
| `session: Option<i64>` | the attached session, or `None` for a draft |
| `ownership: Ownership` | the showing zone, or `None` after the last handover |
| `main`, `resume`, `tree` | the three zones; private to the app |
| `carried: Vec<Request>` | requests taken from a zone that just lost the screen |

### Startup

`Startup` is an open parameter object:

| field | meaning |
|---|---|
| `initial: Option<ZoneId>` | the first showing zone; `None` means `Main` |
| `session: Option<i64>` | a session to attach after the first frame; `None` means a draft |
| `cwd: PathBuf` | the directory a picker or a draft starts in |

`Startup::main()` is the default. `Startup::resume(cwd)` sets `initial` to `Resume`. A new caller
adds a field instead of adding an enum variant.

### Identity

`Uuid` is a 16-byte value built from the current time and a process-local counter. It has no
dependency on a UUID crate. `App::id`, `App::born`, and `App::died` expose the same value; the
loop is expected to register and unregister it with the session.

### Memo

`Memo` is a `HashMap<String, String>` behind interior mutability. `Operator::Write`, `Read`, and
`Delete` act on one alias. The app never validates content. The memo is open only while
`Ctx.memo` is `Some`, which happens only inside `hand_over` and `take_over`.

### Lifecycle

`attach(id)`:

1. Return if the id is already attached.
2. Detach the old session if there is one.
3. Clear the memo and record the new id.
4. Broadcast `attach` to all three zones.
5. Warm the history zone's languages.

`detach()`:

1. Broadcast `detach` to all three zones.
2. Clear the session, memo, and carried requests.

Data is set before the display broadcast. A zone that was not attached still receives `detach`.

### Handover

`hand_over(h)`:

1. Take the old showing zone.
2. Open the memo context.
3. Call the old zone's `hand_over`.
4. Append `h.requests` to `carried`.
5. Register `h.to` as the showing zone.
6. Call the new zone's `take_over` with the same memo context.
7. Assert in debug builds that the old zone's queue is empty.

The app has no pending queue of its own.

### Delivery and drawing

- `deliver_event(ev, ctx)` sends the event to the showing zone only.
- `deliver_action(action, ctx)` sends the action to the showing zone; a returned handover is
  executed immediately.
- `render(ctx)` asks the showing zone for rows.
- `cursor()` asks `Main` for the hardware cursor.
- `take_requests()` returns carried requests first, then the showing zone's own.

## 2. Design intent

- **One ledger, no side doors.** Every screen change is a `Handover`; no zone switch bypasses the
  ledger.
- **The app owns no zone internals.** It talks through `Zone`; the three fields are private.
- **The memo is a transaction, not a bus.** Cross-zone facts exist only while the screen is being
  transferred, so no ordinary delivery can observe a half-written handover.
- **Requests travel with the losing zone.** A zone that will not be asked again cannot keep work
  in its own queue.
- **Startup is open.** New launch parameters do not force a closed enum or a second constructor.
- **Identity is local and dependency-free.** The session can accept the bytes without depending
  on the TUI's app module.

## 3. Unresolved

1. `Startup.session` is stored but not consumed; `App::new` always starts with `session = None`.
2. `App::born` / `App::died` are exposed but no loop registers or unregisters the identity.
3. `set_tree_config` has no caller, so `cherishYourWords` cannot be configured.
4. The frame loop that should call `deliver_*`, `render`, and `take_requests` does not exist.
5. The first frame of a `Resume` start has no size until an event or action arrives.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme` | palette passed to the history and input renderers |
| `config.tui.renderMargin` | history focus margin and overscan |
| `config.tui.todoFold` | history task-list folding |
| `config.tui.usage.align` | usage panel placement |
