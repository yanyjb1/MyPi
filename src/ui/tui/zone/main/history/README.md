# history — the transcript the reader scrolls

## 1. Design / Schema

### Three levels of text

| level | what it is | depends on |
|---|---|---|
| segment | a run of source lines of one kind: prose, fenced code, table, quote, list, a hard-broken paragraph tail | nothing |
| natural line | one source line inside a segment | nothing |
| width row | a natural line folded to the terminal width | the width |

A segment never shares state with another segment. A natural line carries its own color slots, so no line needs its neighbours to be explained.

Prose rows carry a lead marker: `▮ ` on a natural line's first row, `↳ ` on a row that only exists because the line was folded. The marker is two columns of `fixed::DIM`. Cards supply their own frame and carry no lead.

### Blocks

`shape::Block` is one row of the block table: id, kind (user, wake, assistant, reasoning, tool request, tool result), text, name, arguments, intent, call id, whether it opens its round, success, result, memo, duration, usage.

Two classes of block:

| class | blocks | taken by |
|---|---|---|
| markdown | assistant, reasoning, user | the segment |
| tool | tool request, tool result | the whole block |

### Slots

`local::Local` is one append-only run of numbered slots. Numbers below zero are the past, newest at -1; zero upwards are this conversation's messages. A slot holds a block id, a notice, or `Next` (a dead short-lived notice: no rows, no gap). A rewind truncates the positive side and frees the numbers.

### The live tail

`live::Live` holds the blocks that have no id yet and are still growing. It keeps, per block, the finished segments' natural rows and a syntax painter for the open fence. It always sits below the last slot.

A round's `wake` pieces land here one block each, so every reminder keeps its own text. A message this front end put out is echoed here too: it leads the tail, above the reply, and a `retract` takes it back.

### The handover

When a round closes, its structure arrives as id ranges. If the tail holds exactly as many blocks as the ranges cover, the tail hands its product over instead of being dropped: each block takes its id in order, becomes a numbered slot, and the region above never reorders. Anything the tail never carried — and the fields only the stored row has — still comes back from `call blocks`; a row that differs from the handed-over block drops that block's cached rows.

### The cache

| layer | holds | dropped when |
|---|---|---|
| parsed | block → its segments' natural rows | the block is re-rendered or evicted |
| width rows | block → rows at the current width, plus the row index of every natural line | the width, the two fold switches or the fold budget change |

Colors are not cached. Both layers are capped by `config.ui.tui.renderMargin` blocks; the earliest admission goes first.

### The viewport

`view::Viewport` holds the sight line (a screen row, default 3), the seam and the rows the host allocated. The seam is either `End` (following the newest content) or an anchor: a slot number and a spot. A spot is a natural line plus the cells before it, so a resize re-finds the row instead of guessing.

One frame is assembled by `compose`: the seam's own row splits the window; above it, slots are walked older until the sight line's budget is spent; below it, the seam's tail, newer slots, then the live tail. Blank rows separate slots and count against the height.

### Folding

The thinking chain has its own copy (`Ctrl+T`) and so do tool cards (`Ctrl+O`). A folded run keeps its first row, a placeholder row and the newest `budget - 2` rows; a running tool keeps its newest `budget` rows. The budget is `config.ui.tui.history.cardFoldBudget`.

### Keys

| key | does |
|---|---|
| `Ctrl+T` / `Ctrl+O` | flip the thinking chain's / tool exchanges' copy |
| `Ctrl+Shift+Home` / `End` | first / last block of the sight line's round |
| `Ctrl+PageUp` / `PageDown` | first block / follow the newest |
| `PageUp` / `PageDown` | a screen older / newer |
| `Ctrl+↑` / `Ctrl+↓` | one slot older / newer |
| `Shift+wheel` | a screen older / newer |
| wheel | one slot older / newer |

### What it shouts

`busy` (on for any round's first piece, off when a round closes), `usage` (`{used}` on a block row
that carries a token count) and `prefill` (`{text}` after a rewind). It hears `cwd`, `echo` and
`retract`; the last two are the front end's own message and its withdrawal.

## 2. Design intent

- **A number, once handed out, never moves.** Landing keeps the order; only a rewind takes the tail.
- **Segments are the unit that survives a resize.** A width change re-folds the natural lines; it never re-parses them.
- **The reader's place is a natural line, not a row.** A resize cannot push the sight line into another sentence.
- **Predict, do not measure.** Rows for slots nobody looks at are not laid out; their height is the last one known.
- **Colors resolve at the exit.** The palette is one accent plus constants, and a switch costs one repaint of the visible rows.

## 3. Unresolved

1. Streaming animations per tool: only the finished layout exists.
2. Tool renderers for `context`, `memo`, `kairos`, `mcp`, `alias` are the fallback card.
3. Every visible slot's rows are cloned per frame.
4. A block holding a table skips the parsed layer, because column widths depend on the width.
5. A wake reminder has no style of its own: it is drawn as a notice row.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.renderMargin` | blocks kept in each cache layer |
| `config.ui.tui.todoFold` | task-list folding policy |
| `config.ui.tui.history.focusLine` | the sight line, in screen rows above it |
| `config.ui.tui.history.cardFoldBudget` | rows a card head or tail may take before folding |

The thinking placeholder has no config key.
