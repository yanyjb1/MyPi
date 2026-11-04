# history — the transcript the reader scrolls

## 1. Design / Schema

The decisions behind the three layers are recorded in `docs/display-design.md`.

### Three layers

| layer | holds | depends on |
|---|---|---|
| product | a block as pieces: segments and their natural lines, colored | nothing but the block |
| prefetch | the rows laid around the sight line | the width, and the whole piece each row came from |
| display | where the window starts | the reader |

A product is one block's answer to "what am I made of":

| kind | a product is | folding |
|---|---|---|
| text (assistant, reasoning) | segments, each a run of natural lines | the prefetch layer folds them |
| whole-block (tool card, user, wake) | the rows themselves, cut to a width | already cut |

A whole-block product keeps a width, so a resize drops it; a text product keeps none, so a resize
leaves it alone.

### Pieces, rows, numbers

One block is a run of pieces in source order. Prose, code, a quote, a list and a hard break are one
piece each; a table is a piece too, but its columns are only known at a width, so it is laid when the
prefetch layer reaches it. A blank row separates neighbouring pieces, and neighbouring blocks the
same way.

The prefetch layer keeps a run of rows: contiguous, ordered, and each one remembering the piece it
came from. It is built outwards from wherever the reader is, never over the whole conversation.

### The display layer

The window is a slice of that run. Two states:

| state | the window | new content |
|---|---|---|
| follow (`top` is `None`) | the last rows there are | pushes the top out, the reader stays at the bottom |
| frozen (`top` is a piece) | starts at the row that piece begins with | lands behind the reader, nothing moves |

A scroll moves the start of the window by `scrollHeight` screen rows, or by a full window for a page.
Rows already laid are reused; only the rows that walk past either end are laid. A scroll towards the
newest content that runs out of content goes back to following.

The sight line (`focusLine`, default 3) is where a block's head is put when the reader asks for one,
and where the anchor for "keep this turn's neighbours in hand" is read.

### Width

A width change drops every laid row and every whole-block product. Text products survive: their
natural lines are re-folded, the block is not parsed again.

### Blocks and the local section

`wire::decode::Block` is one row of the block table: id, kind (user, wake, assistant, reasoning, tool
request, tool result), text, name, arguments, intent, call id, whether it opens its round, ok, result,
memo, duration, usage.

`local::Local` is an append-only run of numbered slots. Numbers below zero are the past, newest at
-1; zero upwards are this conversation's messages. A slot holds a block id, a notice, or `Next` (a
dead short-lived notice: no rows, no gap, no budget).

`wire::live::Live` holds the blocks that have no id yet and are still growing; the prefetch layer
treats it as the piece right after the newest slot, and re-lays it in place whenever it grows.

### Tool cards

A tool block becomes either a card or a one-line form; `card::node` decides by tool name.

| form | tools |
|---|---|
| card | `bash`, `edit`, `write`, `todo`, `web`, `context`, `memo`, `mcp call`, `kairos result`, `final` |
| one line | `read`, `cd`, `alias`, `task`, `kairos` (every op but `result`), `mcp list`, `final submit` |

A card is a frame: a top edge carrying the header (`status glyph`, bold tool name, then the title
the renderer names), one section per call body and per result, and a bottom edge. The tail is a
table when the tool answers rows: `card::Context`, `card::Memo` and the rest lay their table with
`paint::table::cells` at the card's inner width.

A one-line form is the status glyph, a bold label, and the values the call carried; `alias list`,
`kairos list`, `mcp list` and `task` add `tree` rows under it (`|-- ` for a middle entry, `'-- `
for the last, the spine under a non-last entry).

Renderers read the result's memo, not its text: a renderer with no memo falls back to the same
result text every tool prints.

### The handover

When a round closes, its structure arrives as id ranges. If the tail holds exactly as many blocks as
the ranges cover, the tail hands its product over instead of being dropped: each block takes its id
in order and becomes a numbered slot. Ranges the tail did not cover come back from `call blocks`; a
row that differs from the handed-over block drops that block's product.

### Keys

| key | does |
|---|---|
| `Ctrl+T` / `Ctrl+O` | flip the thinking chain's / tool exchanges' copy |
| `Ctrl+Shift+Home` / `End` | first / last block of the sight line's round |
| `Ctrl+PageUp` / `PageDown` | oldest slot / follow the newest |
| `PageUp` / `PageDown` | a window older / newer |
| `Ctrl+↑` / `Ctrl+↓` | one slot older / newer |
| wheel | `scrollHeight` rows |
| `Shift+wheel` | a window |

### What it shouts

`busy` (on for any round's piece, off when a round closes), `usage` (`{used}` on the increment that
carries a request's bill) and `prefill` (`{text}` after a rewind). It hears `cwd`, `echo` and
`retract`; the last two are the front end's own message and its withdrawal.

## 2. Design intent

- **A width is the prefetch layer's business alone.** Products are built once per block; only the
  folding knows how wide the terminal is, so a resize costs one layer and no parsing.
- **Take whole pieces, drop whole pieces.** A row is never half a piece, because a fence's colors and
  a list's indentation are decided per segment.
- **Predict nothing.** Nothing asks how tall an unseen block is: the window is always derived by
  walking outwards from where the reader stands.
- **A number, once handed out, never moves.** Landing keeps the order; only a rewind takes the tail.
- **Colors resolve at the exit.** A row carries color slots, not colors, so the accent costs one
  repaint of the visible rows.

## 3. Unresolved

1. Streaming animations per tool: only the finished layout exists.
2. `web` still falls back to the fallback card.
3. Every visible row is cloned into a ratatui `Line` per frame.
4. A wake reminder has no style of its own: it is drawn as a notice row.
6. Consecutive `read` calls are not folded into one tree: the paint path draws one block at a time.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.zone.main.history.cacheMargin` | blocks kept in the product layer |
| `config.ui.tui.zone.main.history.focusLine` | the sight line, in screen rows above it |
| `config.ui.tui.zone.main.history.cardFoldBudget` | rows a card head or tail may take before folding |
| `config.ui.tui.zone.main.history.scrollHeight` | screen rows one wheel notch walks |
| `config.ui.tui.zone.main.history.todoFold` | task-list folding policy |
