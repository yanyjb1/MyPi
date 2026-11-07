# history — the transcript the reader scrolls

## 1. Design / Schema

Four parts: the local section, the product layer, the prefetch layer, and a display step.

| part | holds | hands down | evicts |
|---|---|---|---|
| local section | positions, and the content behind them | a position, and the block it names | never; only a rewind empties a position |
| product layer | a block as segments, or as folded rows | segments, or rows | by block, `cacheMargin` of them |
| prefetch layer | laid segments and gaps | rows, in screen order | by segment, whole segments only |
| display | nothing | screen lines | nothing |

### Local section

A position is either `Fixed(i64)` or `Turn(u32)`.

`Fixed` numbers the finished conversation: `-N..-1` is the restored past, newest at `-1`; `0..M` is
this front end's own run. `Turn` numbers the entries of the running round while it grows; those
numbers do not survive the handover.

A position holds one of three things: a block id, a line this front end says (a notice), or `Next`.
`Next` keeps its number, draws nothing, and takes no budget.

Content is one `Block`: id, kind, text, name, arguments, intent, call id, ok, result, memo,
duration, and whether it opens its round. Blocks are stored by id, not by position.

The running round is an ordered list of entries; each entry is a block or a notice. A block entry
takes an upstream id when the round's receipt arrives; a notice never does. The receipt is checked
by type first: the number of ids must equal the number of block entries, notices are not counted.

### Product layer

Two lifetimes:

| layer | content | on a width change |
|---|---|---|
| first | segment to natural lines | untouched |
| second | natural lines to folded rows | discarded and rebuilt |

Two product kinds: text segments (prose, thinking, fences) and whole-block products (cards, user
messages, notices, tables, task lists). A text product is a list of segments; a whole-block product
is rows already folded at a width.

The cache is keyed by block id. Eviction compares admission time first, then the id; `Next`
positions have no product, and a running round is never cached as a block. A width change drops every
whole-block product and every laid row; text products survive.

A running block is rebuilt whenever content arrives. Only its last segment can still change.

### Prefetch layer

The unit is the segment: it enters whole and leaves whole. A segment is colored once, then kept with
the rows it took.

Segments are laid in render order. A named position is `SegId { idx, piece }`. A row after each
segment is a gap; the gap belongs to the segment's tail.

A segment whose content is not in hand draws one `…` row and reports the block id; nothing is
cached for it.

The display range is a start plus the rows the screen has. Two starts:

- `Latest`: the last rows of the run. New content lands on screen by itself.
- `At(SegId, row)`: one fixed place. The screen does not move when content lands elsewhere.

### Display

The display turns laid rows into screen lines: it resolves each color slot to a terminal color,
puts the `▮` or `↳` mark in front of the row, and emits the line. It holds no state.

### Rows and marks

One row is a list of runs; a run is text plus an optional foreground slot, an optional background
slot, and modifiers. A row also carries a lead: `None` (the block draws its own edge), `First` (it
opens a natural line), or `Wrapped` (it is a continuation).

## 2. Design Intent

- **Indexes and content are separate** so that a position can exist before its content arrives. The
  view can name anything in the structural window; content is fetched on demand.
- **Blocks keep ids, not positions**: a rewind changes positions, never ids.
- **`Next` keeps its number** so that a rewind does not renumber anything the reader has already
  seen.
- **The segment is the unit** because coloring depends on the whole run: a fence's syntax state and
  a paragraph's spans cross row boundaries.
- **Gaps belong to the segment's tail** so that laying one segment never has to look at its
  neighbor.
- **Folding lives in the product layer** because it depends on width, while segments do not.
- **The range is a start plus a height** so that the screen is a slice of the run, and a growing
  block only changes the rows after its last settled row.
- **Display is stateless** so that the same rows always produce the same screen.

## 3. Unresolved

- The structural snapshot arrives through the host after `Request::Structure`; the zone lays its
  positions from it.
- Block content arrives through `Request::Blocks`; the zone asks per round, once per range.
- The checklist is drawn from `call todo` and `call init`, pinned under the transcript.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.zone.main.history.cacheMargin` | blocks kept in the product layer |
| `config.ui.tui.zone.main.history.focusLine` | rows above the sight line when a block is focused |
| `config.ui.tui.zone.main.history.cardFoldBudget` | rows a card head or tail may take |
| `config.ui.tui.zone.main.history.scrollHeight` | rows one wheel notch walks |
| `config.ui.tui.zone.main.history.todoFold` | task-list folding policy |
