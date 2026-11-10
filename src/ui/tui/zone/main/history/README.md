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

Content is one `Block`: id, kind, the text or arguments it carries, name, intent, call id, the
result and its ok, memo, duration, and whether it opens its round. A tool exchange is two blocks: a
request and the result that answers it.

A request finds its result by call id, never by position: the upstream writes every request of a
round before any of its results, so two calls in one round sit next to each other. The pairing index
lives in the local section; the running round and the finished conversation both answer it.

The running round is an ordered list of entries; each entry is a block or a notice. A block entry
takes an upstream id when the round's receipt arrives; a notice never does. The receipt is checked
by count first: the number of ids must equal the number of block entries, notices are not counted.
A receipt that does not line up means the turn was cut short — a call that never ran leaves no
block behind — so the running round keeps the screen while the receipt's own rows are fetched;
those rows then lead, positions are taken from them in id order, and only then does the running
round go. While that wait lasts, the next round's entries grow beside it and take over on arrival.

### Product layer

Two lifetimes:

| layer | content | on a width change |
|---|---|---|
| first | segment to natural lines | untouched |
| second | natural lines to folded rows | discarded and rebuilt |

Two product kinds: text segments (prose, thinking, fences) and whole-block products (cards, user
messages, notices, tables, task lists). A text product is a list of segments; a whole-block product
is rows already folded at a width.

The cache is keyed by block id and the result id it pairs with, and a block keeps one entry: a
result that lands later replaces the copy built without it. A product that would draw nothing is not
kept; eviction compares admission time first, then the ids; `Next` positions have no product, and a
running round is never cached as a block. A width change drops every whole-block product and every
laid row; text products survive.

A running block is rebuilt whenever content arrives. Only its last segment can still change.

### Tool cards

A tool exchange draws as one card, built from the request and the result it pairs with. The card
shows one of three phases: `Pending` (no settled result yet), `Ok`, `Err`. The phase picks the edge,
the fill and the glyph; a call the user interrupted or a deadline cut short is `Err` like any other
failure, and its card body is the error color end to end. The file a call names sits in the header
in the path color, red when the call failed.

A live result that was cut adds one row after the family's own rows, outside every fold:
`[N lines & M truncated, see the #ref later]`, where M is the full text's byte count in KB or MB. It belongs to the live block alone; once the
result lands, the stored text carries the same fact in its last line.

The header prints the call's `intent` after the colon, as the request carried it.

The call half is read from the arguments as they stream: `partial` decodes the fields a fragment has
reached, so an edit draws its `old`/`new` text and a write its numbered content before the arguments
finish. Only a key that is itself complete is trusted; a value cut mid-string is drawn as far as it
arrived.

An `edit` result is drawn from its memo plus its arguments: the memo names what the request cannot
(the line an `exact` match sat on, the text a `mass` range replaced) and the arguments carry the
rest. A call with no diff memo — `final`'s `edit` — draws the same shape from its arguments alone.

A diff line carries its text in one color end to end — marker, number, bar and body — and only the
run that changed is reversed, with the indentation never part of it. Its sign and line number are
padded together, so the bar sits in one column; a number the line above already printed is left out.
A line-number jump inside one side is one `…` row. A tab takes three columns wherever a gutter or a
code body is drawn, and the indentation of a diff line or a writer's preview draws one dim glyph per
space and a dim arrow per tab.

The call half of a `todo` call is one row: the phase glyph, the operation, the numbers a `tick`
named with their item text, and the counts the result carries. A `tick` with `done: false` reads as
`untick` and leaves the text uncrossed. The checklist itself is the pinned product, never part of
the card.

### Prefetch layer

The unit is the segment: it enters whole and leaves whole. A segment is colored once, then kept with
the rows it took.

Segments are laid in render order. A named position is `SegId { idx, piece }`. A row after each
segment is a gap; the gap belongs to the segment's tail.

A segment whose content is not in hand draws one `…` row and reports the block id; nothing is
cached for it.

The run keeps one reserved tail slot: the pin, a list of rows laid under the newest segment. It is
not a segment and takes no `SegId`; `len` and `row` count it, `trim` and `at` do not. The zone
rebuilds it every frame from the checklist and from what is still running, so a round close never
has to clear it.

The pin is the checklist, then the ESC row while a call or a thought runs. It scrolls like any other
row: it is not an overlay, and going up walks into the transcript above it. The ESC row reads
`spinner ESC <elapsed> · <label>`; the label is the intent of the earliest unfinished call, or
`thinking` while only reasoning arrives, swept by the accent at the speed content arrives.

The display range is a start plus the rows the screen has. Three starts:

- `Latest`: the last rows of the run. New content lands on screen by itself.
- `At(SegId, row)`: one fixed place. The screen does not move when content lands elsewhere.
- `Pin(row)`: a row of the pin. It only happens when the screen is shorter than the pin itself.

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
- **A pair is a call id, not a neighbour** because a round commits every request before any result,
  so the call and its answer are not adjacent.
- **`Next` keeps its number** so that a rewind does not renumber anything the reader has already
  seen.
- **The segment is the unit** because coloring depends on the whole run: a fence's syntax state and
  a paragraph's spans cross row boundaries.
- **Gaps belong to the segment's tail** so that laying one segment never has to look at its
  neighbor.
- **Folding lives in the product layer** because it depends on width, while segments do not.
- **The range is a start plus a height** so that the screen is a slice of the run, and a growing
  block only changes the rows after its last settled row.
- **The pin is a row set after the segments, not a layer above them** so that one scroll model
  covers the checklist and the transcript, and a short screen still shows the checklist.
- **The pin is rebuilt every frame** so that no handover, width change or round close has to know
  about it.
- **Display is stateless** so that the same rows always produce the same screen.
- **A card is drawn from both halves of its exchange**, so the request's position owns the rows; a
  result landing re-lays that position, not only its own.
- **A card is rebuilt from the stream it has**, so partial arguments produce a partial card and
  nothing waits for the call to finish.

## 3. Unresolved

- The structural snapshot arrives through the host after `Request::Structure`; the zone lays its
  positions from it.
- Block content arrives through `Request::Blocks`; the zone asks per round, once per range.
- The checklist follows `call init` and the memo of every todo result; nothing asks for it
  separately.
- The full text of a truncated result lives in `artifacts`; the front end has no way to read it, and
  a stored block carries no truncation field, only the result text's last line.
- `write` carries no memo, so its card can only preview the new content; there is no old file to
  diff against. An `edit` card draws its diff from the request plus one memo field, so context
  lines outside the replaced range are not shown.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.zone.main.history.cacheMargin` | blocks kept in the product layer |
| `config.ui.tui.zone.main.history.focusLine` | rows above the sight line when a block is focused |
| `config.ui.tui.zone.main.history.cardFoldBudget` | rows a card head or tail may take |
| `config.ui.tui.zone.main.history.scrollHeight` | rows one wheel notch walks |
| `config.ui.tui.zone.main.history.todoFold` | task-list folding policy |
