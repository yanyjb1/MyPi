# history — the transcript the reader scrolls

The history sub-zone is the conversation on screen. It decodes every block row, keeps the rows
around the reader's sight line, paints blocks on demand, and asks the session for what it does not
hold.

## 1. Design / Schema

### Four parts

A **block table** (`id → Block`, decoded), a **live tail** (the running turn's blocks, no ids), a
**local section** (numbered slots, in order), and a **viewport** (the seam and the walk). A **cache**
keeps the rows already painted.

### Numbers

One append-only run of slots. Numbers below zero are the past, counting back from -1; zero is the
first message of this conversation, new ones count up from there.

A slot holds a block id, a notice, or `Next` (a dead short-lived notice: no rows, no gap). Attach
lays the whole structure out as the past, newest block at -1. A closed round appends its blocks at
zero and up. A rewind is the one thing that takes slots away: the structure stops owning the tail,
the slots go, and their numbers are handed out again. The reply's `cut` is the tree's; this zone
reads the structure, never the cut.

### The live tail

Upstream's pieces (`text`, `thinking`, `arguments`, `boundary`, `tool_start`, `tool_progress`,
`tool_end`) are gathered into blocks with no id: the reply's text, the thinking chain, a tool
request per call id, its result per call id. The tail renders under the last slot and is dropped
whole when the round's `ranges` arrive; the blocks that land then come from `call blocks`.

### The sight line and the viewport

The sight line is a seam between two rows, `focus_line` rows below the top of the zone. The anchor
is a slot plus a row inside it, never a pixel and never a line offset.

Rows are walked out from the seam: older above, newer below. A slot with rows takes them; one blank
row follows it. `Next` is stepped over and does not break that separator. Two rows are pinned last:
the task list.

At the bottom the seam is one past the newest slot, so the newest content sits against the pinned
rows: a growing block pushes its own head off the top instead of moving the anchor.

### Segments

A block's prose is split on source lines into `Text`, `Code`, `Table`, `Quote`, `List`, `Break`
runs. Each segment renders on its own: prose through markdown, code through syntect, wrapped to the
width. A segment is the smallest thing painted and the only unit a later re-layout or a backwards
walk can start from.

### The cache

Per slot: the rows painted, keyed by width, working directory, theme epoch and the two fold
switches. Any key change drops everything. At most `margin * 2 + 1` slots keep rows; the earliest
admission is evicted first.

### Folding

Two copies per block, one at a time: `Ctrl+T` is the thinking chain's own copy, `Ctrl+O` the tool
exchanges'. A running tool's card shows its tail, up to the rows the zone has, with a
`… N earlier lines` marker above it; a finished one folds from the head. Expanded shows everything.

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

`busy` (on a live piece: `{on: true}`; when a round closes: `{on: false}`), `usage` (`{used}` after
any block row that carries a token count) and `prefill` (`{text}` when a rewind puts a message
back). It hears `cwd`.

## 2. Design intent

- **A number, once handed out, never moves.** Landing keeps the order; only a rewind takes the tail.
- **The reader's place is a seam, not an offset.** A resize invalidates every offset and no seam.
- **Predict, do not measure.** The reader's moves say what to paint; blank space is cheaper than
  painting a screen nobody asked for.
- **Prose is parsed once per segment.** A segment that did not change is never re-parsed.
- **Only the tail is live.** The running turn is painted directly; nothing about it is cached.

## 3. Unresolved

1. A live block is re-painted whole on every piece, not segment by segment.
2. Rows are never trimmed by distance from the seam; the cache budget alone decides.
3. A resize keeps the anchor, not the exact row: the cut-preserving re-layout of the sight-line
   block is not built.
4. The thinking chain is no longer pinned; it renders in place.
5. Terminal-too-small behaviour is clamping, not a designed degradation.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme` | colors for text, cards, code, and the tree |
| `config.tui.renderMargin` | the cache budget around the sight line |
| `config.tui.todoFold` | task-list folding policy |

`focus_line`, the fold starting values and the thinking placeholder have no config keys yet.
