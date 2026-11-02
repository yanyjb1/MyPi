# tree — the block tree

A full-screen navigator over the blocks of one conversation: one block, one row.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `structure: Structure` | the last structural window handed down |
| `blocks: BTreeMap<i64, Node>` | what each block says, as the answers carried it |
| `rows: Vec<Row>` | the projection, rebuilt only when the inputs moved |
| `selected` / `scroll` | position inside `rows` |
| `only_user` | `s` keeps user messages only |

`Node` is the projection of one block: its kind in the tree's words, a one-line summary, and the
body when it is a user message. `Row` adds the geometry the layout computed.

### Rows

`source::rows` walks the structure's ranges, joins each block id with its node (a block the
window did not carry still gets a row, without a summary), filters `only_user`, then lays the
result out with `layout::rows`. The geometry is a single fork today.

### Keys

| key | does |
|---|---|
| `↑` / `↓`, `PageUp` / `PageDown`, `Home` / `End` | move |
| `s` | toggle user messages only |
| `Enter` | rewind to the row; `Shift+Enter` focuses it in the transcript instead |
| `Esc`, `q` | back to `Main` |

Focusing writes `tree.focus` = `turn:block` into the handover memo; `Main` reads it on start.

### Requests

`start` asks for the structural window and the first load; `Enter` (rewind) sends `post rewind`.

## 2. Design intent

- **Structure plus content.** The ranges say which blocks exist and which round owns them; the
  answers say what they say. The tree stores both, joined on the block id.
- **Rebuild lazily.** A flag says the rows are stale; the draw rebuilds them once.
- **Leaving a note, not a call.** The tree tells `Main` where the cursor was through the memo.

## 3. Unresolved

1. Multi-fork projection: the geometry supports it, the data source does not exist.

## 4. Config fields

None.
