# tree — the block tree

`Tree` is a full-screen navigator over the blocks of one conversation. One block is one row. The
tree exists to pick a point to rewind to, or to put the transcript's sight line on a block.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `size: Option<TermSize>` | the last terminal size |
| `rows: Vec<Row>` | the projected tree |
| `selected: usize` | the highlighted row |
| `scroll: usize` | the first visible row, written back by the renderer |
| `pending: Vec<Request>` | rewind and range requests |
| `only_user: bool` | `S` shows only user messages |
| `focus_on_handover: bool` | Enter asked to focus the transcript, not rewind |
| `requested: bool` | the range was already asked for this takeover |
| `cherish: bool` | `cherishYourWords`: Enter focuses, Shift+Enter rewinds |

### Rows

A `Row` holds:

| field | meaning |
|---|---|
| `geom: RowGeom` | indent, connector, and ancestor gutters |
| `block_id: i64` | the block |
| `turn: i64` | the round that showed it |
| `text: String` | display text, `role: summary` |
| `body: Option<String>` | the full text when the block is a user message |
| `fork: i64` | fork id; currently unused |
| `kind: String` | `user`, `assistant`, `reasoning`, `tool_call`, or `tool_result` |

### Projection

`source::rows(view, only_user)`:

1. Collect held blocks from `view.chain()` by id.
2. Build nodes from every shaped block in `view.shapes`, then from landed blocks in
   `view.blocks`; a node carries its round and, if held, its content.
3. Filter non-user roles when `only_user` is set.
4. Summarize each block as the first line of its content, text, result, or args; a missing block
   shows `role: #id`.
5. Feed the nodes and one fork to `layout::rows`, then attach the row fields.

`source::range(view)` returns the full shaped block-id span, with an exclusive end.
`source::current(view, rows)` finds the newest landed block's row.

### Layout

The geometry is a preorder walk of a flat sequence:

- one child: the child stays in the parent's column;
- several children: the children indent one level;
- siblings use `├─` and `└─`;
- an ancestor's fork draws `│` down to the end of that branch.

Each level is three columns. `RowGeom` stores the indent, the connector, and the gutters; the
renderer turns it into characters.

### Opening and syncing

`take_over` sets the size, syncs from the view, finds the current row, calls `begin(rows,
current)`, and asks for the range once. `begin` resets the scroll.

`sync` rebuilds rows, keeps the selected block if it still exists, clamps the selection and
scroll, and asks for the range if it has not been asked for.

### Range

`Request::TreeRange { start, end }` asks the session for raw material over a block-id range. The
zone asks once per takeover; `requested` gates repeats.

### Keys

| key | does |
|---|---|
| `↑` / `↓` | one row |
| `PageUp` / `PageDown` | ten rows |
| `Home` / `End` | first / last row |
| `S` | toggle user messages only |
| `Enter` | rewind, or focus when `cherish` is on |
| `Shift+Enter` | focus, or rewind when `cherish` is on |
| `Esc` / `q` | return to `Main` |

Enter always hands the screen back to `Main`. A rewind queues `Request::Rewind` with the row's
turn, block, and body. A focus sets `focus_on_handover`; `hand_over` writes `turn:block` into the
memo alias `tree.focus`, and `Main::take_over` passes it to `HistoryZone::focus`.

### Rendering

One row per block, prefixed by the tree geometry. The role name takes the role's color; the
summary takes the body color. Text is truncated by display columns, not character count. The
selected row is reversed across the full width. The top row is a fixed key hint.

### Invariants

1. One block is one row.
2. The tree never owns a second structure cache; rows are projected from the session view.
3. A range is asked for once per takeover.
4. A focus fact leaves through the memo, not through a request.
5. A rewind names both the turn and the block.

## 2. Design intent

- **The session's structure is the source.** The tree projects rows; it does not keep a parallel
  tree.
- **One row per block.** The tree is for choosing a point, not for reading the conversation.
- **Geometry is a pure function.** The layout can be tested without a terminal.
- **A focus is not a request.** The target block belongs to the transcript, so it travels through
  the handover memo.
- **Rewind carries the body.** The body is the text the editor can re-offer; it rides with the
  request instead of being looked up later.
- **`S` is a view filter.** It changes the projection, not the session.

## 3. Unresolved

1. `TreeAsk::Range` exists but is unused; the zone pushes `Request::TreeRange` directly.
2. `Request::TreeRange` has no session-side landing call.
3. `source::rows` feeds one fork with no parent, so the multi-fork geometry is not exercised by
   the current projection; `Row.fork` is always zero.
4. `cherishYourWords` has no config key; `set_config` is exposed but no caller sets it.
5. A resize resets the scroll to zero instead of keeping the selected row in place.
6. The range request is gated by `requested`, but a changed session does not reset it until
   `detach`.

## 4. Config fields

None. `cherishYourWords` is a future config switch; see Unresolved.
