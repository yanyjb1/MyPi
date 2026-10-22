# history — the transcript the reader scrolls

The history sub-zone is the conversation on screen. It numbers every message, keeps the rows
around the reader's sight line, paints blocks on demand, and asks the session for more when the
reader moves.

## 1. Design / Schema

### Three parts

A **local section** (what the conversation is made of, in order), a **viewport** (which slots are
on screen and where), and a **cache** (split products, laid-out rows, and heights). The session
owns the structure and content; this zone owns only what it has painted.

### The local section

One append-only sequence of slots. A slot holds exactly one of four things:

| slot | holds |
|---|---|
| `Empty` | the number is taken; the block id has not arrived |
| `Block(id)` | one block, named upstream by that id |
| `Notice { text, level, life }` | a line this front end has to say |
| `Next` | a dead short-lived notice: no rows, no separator |

Numbers below zero are the past, counting back from `-1`; `0` is the first message of this
conversation, and new ones count up from there. A number is handed out once and never changes
meaning.

The split is whether a slot has a block id. Slots without one form the mutable region at the
bottom, after the immutable one. When a round ends and its block ids arrive, the empty slots land
in order and keep their numbers. A number is never dropped; the content behind it may be.

A short-lived notice dies when the next block lands or another short-lived notice arrives. Its
slot keeps its number and becomes `Next`, so it takes no row and no separator and the sequence
does not collapse.

The zone does not produce notices. `notice(text, level, life)` is the entry point; the caller
decides when one exists.

### Opening a conversation

`attach(session)` destroys the old local section and asks for the new one with `Ask::Init`. The
first attach is spared the destroy: a conversation that is just being born owns what is already
on screen. It still sends the Ask.

`reset()` is `/new`: it destroys the section and restores the reprieve, so the next attach does
not destroy again.

### The sight line and the viewport

The sight line is a seam between two rows, `focus_line` rows below the top of the zone. It is
where the reader's eyes rest; keeping what is near it still while the terminal resizes is the
point of the layout.

- The anchor is a slot plus a row inside that slot. It is never a pixel or a line offset.
- Rows are laid out by walking slots out from the seam: older above, newer below. A slot with
  something to show takes its own rows; one blank row follows it.
- The bottom is a state, not a position: the newest block sits against the pinned rows. Its seam
  is one past the newest slot, so one step older lands on the newest and one step newer stays.
- Two rows are pinned last in render order: the task list, then the thinking chain.
- A resize keeps the anchor: the same slot and the same row inside it, clamped to the new height.

### Products, layout, and the cache

A **product** is the split result of painting one block: segments of logical lines, cut points,
and deferred code fences. It is not the final screen rows.

A **layout** turns a product into rows at a width: it wraps each logical line, records the row
range of each deferred fence, and keeps the fence's highlighter for later. The `seam` argument is
accepted but does not select a cut point yet.

The cache keeps, per number:

| item | lifetime |
|---|---|
| `products: Vec<Product>` | one or two copies, kept across width changes |
| `variants: Vec<Block>` | laid-out rows, dropped on width, theme, cwd, generation, or landing |
| `stamp` | admission order, used for eviction |

It also keeps `heights: [usize; 2]`, `written` order, and `versions`. Heights survive row drops,
so a walk that only needs a height does not paint again.

- Width change: keep products and versions, drop rows, heights, and write order.
- Theme, working directory, or generation change: drop everything.
- Moved live tail: invalidate the live slots.
- Landing: invalidate the landed numbers so the next pass paints the landed version.
- Eviction: while not busy, keep at most `budget` slots; the oldest `stamp` goes first.
- Trimming: at landing, drop rows outside the sight-line circle and, when the reader is
  elsewhere, outside the bottom circle. Rows go; numbers stay.

### Two copies

One number can hold two copies of a block: the default and the other one.

- Reading takes a line range and whether to read the other copy.
- A tool exchange's other copy is its expanded form; a thinking chain's other copy is its folded
  one-row form.
- The switches are `Ctrl+T` (thinking) and `Ctrl+O` (tools). The starting values come from
  `Config`, are read once, and are never written back.

### The Asks

| Ask | carries | asked when |
|---|---|---|
| `Init` | the session number | the front end attaches |
| `Jump` | one round | the target has no rows and is not the newest round |
| `Fetch` | one block and a side | the side the reader moved toward has fewer than `overscan` painted blocks |
| `Settled` | nothing | the reader jumped and has been still for `rest_ms` |

`Settled` asks nothing back: it tells the session to drop the throttle it keeps for repeated
jumps. Only a jump arms it; any move restarts the count.

### Keys

| key | does |
|---|---|
| `Ctrl+T` | flip the thinking chain's copy |
| `Ctrl+O` | flip tool exchanges' copy |
| `Ctrl+Shift+Home` / `End` | first / last block of the sight line's round |
| `Ctrl+PageUp` / `PageDown` | first block / follow the newest |
| `PageUp` / `PageDown` | a screen older / newer |
| `Ctrl+↑` / `Ctrl+↓` | one slot older / newer |
| `Shift+wheel` | one round older / newer |
| wheel | one slot older / newer |

`Ctrl+Home` / `Ctrl+End` belong to the editor.

### Rendering

One row pass per frame:

1. Project the session's blocks into `Item`s, pairing tool requests with their results.
2. Sync the cache: generation, tail version, width, cwd, theme.
3. Land ids, then trim if something landed.
4. Sync the task list.
5. Plan which held numbers are visible and which are lazy.
6. Lay out the visible numbers immediately.
7. Build pinned rows (task list and thinking chain).
8. Append a running tool's tail after the last slot.
9. Walk the slots around the seam and pad to the allocated height.

### Invariants

1. What reaches the screen is whole blocks; nothing addresses a slice of one as a unit.
2. A number is handed out once, and re-landing a block keeps its number.
3. A block's rows are on screen only if the block is on screen.
4. What is dropped is rows, never numbers.
5. A jump is never asked for a target whose rows are already here, and never for the newest
   round.
6. A resize keeps the seam, not the rows.
7. The task list and the thinking chain are pinned last.

## 2. Design intent

- **A number, once handed out, never moves.** Landing must not renumber the sequence.
- **The reader's place is a seam, not an offset.** A resize invalidates every offset and no seam.
- **Products outlive rows.** A width change can re-wrap from logical lines without asking the
  session for raw material.
- **Two copies per block.** Folding is a read-time switch, not a re-layout.
- **Predict, do not measure.** The reader's moves say what to paint; blank space is cheaper than
  painting a screen nobody asked for.
- **Heights outlive rows.** Scrolling asks for heights more often than rows.
- **Blank space stays blank.** Nothing fills empty rows; the layout does not carry state between
  frames.
- **One Ask per move that needs one.** A scroll asks for a side; a jump asks for a round; neither
  asks when the material is already here.

## 3. Unresolved

1. The frame loop does not exist, so `Init`, `Jump`, `Fetch`, and `Settled` are collected and
   dropped.
2. `Ask::Init` has no `Session::init`; `Session::enter` still uses `Call::Tail`.
3. `resize::plan` computes a lazy list, but `layout::lazy` has no consumer.
4. `layout::layout` accepts a seam but does not use it to choose a cut point; a resize keeps the
   anchor, not the exact row.
5. Products are generated from rendered logical lines at the current width. Cards and tables are
   still width-dependent at generation time, so a width change can only re-wrap what was already
   generated.
6. The task list's folding, the thinking placeholder, and the fold switches have no config keys;
   they use `Config::default` unless `App` overrides the fields it knows.
7. `HistoryZone::notice` has no caller outside the zone.
8. Terminal-too-small behaviour is clamping, not a designed degradation.
9. A long-lived notice does not kill a short-lived one; the design calls for any persistent
   arrival to do so.

## 4. Config fields

| key | use |
|---|---|
| `config.tui.theme` | colors for text, cards, code, and the tree |
| `config.tui.renderMargin` | `focus_margin` and `overscan` |
| `config.tui.todoFold` | task-list folding policy |

The remaining `Config` fields — `focus_line`, `rest_ms`, `on_thinking`, `unfold_thinking`,
`unfold_tool` — have no config keys yet; see Unresolved.
