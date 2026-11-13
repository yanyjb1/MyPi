# history — the transcript the reader scrolls

## 1. Design / Schema

Four parts: the local section, the prefetch layer, a display step, and the frame that decides when
any of it runs.

| part | holds | hands down | evicts |
|---|---|---|---|
| local section | positions, and each position's content plus its product | a position, and what it draws | content past `cacheMargin`, oldest admission first, blocks in use exempt |
| prefetch layer | laid segments and gaps | rows, in screen order | by segment, whole segments only |
| display | nothing | screen lines | nothing |
| frame | the revision the last draw showed | `None` while nothing changed | nothing |

### Local section

A position is either `Fixed(i64)` or `Turn(u32)`.

`Fixed` numbers the finished conversation: `-N..-1` is the restored past, newest at `-1`; `0..M` is
this front end's own run. `Turn` numbers the entries of the running round while it grows; those
numbers do not survive the handover.

A fixed position holds one of three things: a block (with its content when it is in hand), a line
this front end says (a notice), or `Next`. `Next` keeps its number and draws nothing. A block with
no content is one of the restored past whose rows have not been fetched, or one the budget took
back; both draw the `…` row and ask for the block.

Content is one `Block`: id, kind, the text or arguments it carries, name, intent, call id, the
result and its ok, memo, duration, and whether it opens its round. A tool exchange is two blocks: a
request and the result that answers it.

A request finds its result by call id, never by position: the upstream writes every request of a
round before any of its results, so two calls in one round sit next to each other. The pairing index
lives in the finished region; the running round answers from its own entries.

The running round is an ordered list of entries; each entry is a block or a notice, and it carries
the product derived from it. Entries take their places by arrival. A block entry takes an upstream
id when the round's receipt arrives; a notice never does. The receipt is checked by count first: the
number of ids must equal the number of block entries, notices are not counted.

A receipt that does not line up means the turn was cut short: a tool call interrupted before its
arguments were complete never ran, and a request that never ran leaves no row behind. The zone takes
the receipt's numbering at once — one empty position per id — asks the database for those rows, and
binds each row to its own position as it arrives. When the last one is in hand the turn's blocks go
and its notices land after them. Until then the running round keeps the screen and the statusline
stays busy, so nothing else grows beside it.

The two regions are the only holders of block content. What the budget takes away leaves the slot's
number and kind behind, so a later scroll asks for it again instead of keeping it twice.

### Products

One product per block: text blocks hand over segments, everything else hands over rows folded at a
width. A whole-block product is built at the width it will be drawn at; a text product is built
without one.

The product is built on the first use of a position and kept on the block. It goes when the block's
content changes, and when the width, the fold budget or the accent's options move. A product that
would draw nothing is not kept.

The running round rebuilds a block's product when a piece lands: a card is laid again from the
stream it has, and a running card builds only the rows its fold keeps, so a long output costs the
budget, not the output.

A text block does not rebuild. Its product holds the segments already closed and re-splits only the
open tail, so one piece costs the tail, not the block. The tail's rows go and the run lays that
piece again; the closed segments keep theirs. A piece that only lengthens the open line still
re-lays that line, which is the one quadratic left in the layer.

A compaction summary is rows rather than segments: each piece of it drops the product and lays it
again, which is what keeps its two banners outside the summary's own tail.

### Tool cards

A tool exchange draws as one card, built from the request and the result it pairs with. The card
shows one of three phases: `Pending` (no settled result yet), `Ok`, `Err`. The phase picks the edge,
the fill and the glyph; a call the user interrupted or a deadline cut short is `Err` like any other
failure, and its card body is the error color end to end. The file a call names sits in the header
in the path color, red when the call failed.

A result carries one outcome per item its arguments named, and every status the card draws comes
from those outcomes, never from the result's text.

A call that names several items draws one row per item, each with that item's own glyph, and a
failed item's text under its own row. The five tools whose arguments are arrays — `read`, `alias`,
`kairos`, `mcp` and `task` — draw this way; a single-item call draws exactly its one row, so the
shape does not change with the count. Only a failed item's own text is drawn: the successful items'
bodies never enter the card, which is what keeps a batch's file contents out of the history.

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

### A compaction round

The round a compaction writes is bracketed by two banners, and only its summary draws. Its other
block is the instruction the request carried: the record, never a row.

A banner is one row: `>>>---label---<<<`. The arrow runs narrow tier by tier as the width goes —
three, then two, then one — and the rule between them takes what is left; a width that cannot frame
the label draws it bare, cut to the row. The label is `compaction started` for the banner the round
opens with and `compacted` for the one it closes with; on that one the gauge's figures follow as
`compacted · 245.0K -> ~18.9K`, the context before the round and the estimate it left, marked with a
tilde as an estimate. A figure this front end did not measure is left out, and a round it only reads
back never shows the lower figure.

The summary is folded to one row — `summarizing` while it arrives, `summary` once the round is over
— and `Ctrl + O` lays it out as markdown.

Which blocks belong to a compaction round comes from the structural window; the figures come from
this front end's own `call usage` asks, one when the round starts and one when it closes. The brief
window between the two is the only state the round owns.

### Prefetch layer

The unit is the segment: it enters whole and leaves whole. A segment is colored once, then kept with
the rows it took.

Segments are laid in render order. A named position is `SegId { idx, piece }`. A row after each
segment is a gap; the gap belongs to the segment's tail.

A segment whose content is not in hand draws one `…` row and reports the block id; nothing is
cached for it.

The run keeps one reserved tail slot: the pin, a list of rows laid under the newest segment. It is
not a segment and takes no `SegId`; `len` and `row` count it, `trim` and `at` do not. The zone
rebuilds it every draw from the checklist and from what is still running, so a round close never
has to clear it.

The pin is the checklist, then the ESC row while a call, a thought or a compaction runs. It scrolls
like any other row: it is not an overlay, and going up walks into the transcript above it. The ESC
row reads `spinner ESC <elapsed> · <label>`; the label is the intent of the earliest unfinished
call, `thinking` while only reasoning arrives, or `COMPACTING` while the history is being folded. A
thought keeps `ESC` in the accent and sweeps the label; a call keeps the label in the accent and
sweeps `ESC`; a compaction sweeps its own label. The starburst turns one frame per tick and the
sweep walks 30 columns a second on the row's own clock.

The display range is a start plus the rows the screen has. Three starts:

- `Latest`: the last rows of the run. New content lands on screen by itself.
- `At(SegId, row)`: one fixed place. The screen does not move when content lands elsewhere.
- `Pin(row)`: a row of the pin. It only happens when the screen is shorter than the pin itself.

### Frame

One round belongs to the app: it drains what the terminal said, then what the connection said, then
runs the side effects, then draws only if the showing zone's rows changed.

Inside this zone a frame is three steps. Ingest appends to the model — a piece, a block, a receipt —
and records the position it touched in a dirty set; it never lays anything. A text piece is the one
exception: it grows the position's product in place and records the piece the run must lay again,
so a batch of pieces costs its tails, not the block. The tick applies both sets: a dirty position
loses its laid rows and its product, a grown one loses the rows of its tail piece and everything
after it, the content budget is settled against the window the last draw kept, and what the last
draw lacked leaves as a block request. The draw then spends the window and trims the rows around
it; it changes nothing.

The zone counts its own revisions: every entry point that can change the rows bumps one. The app
draws when that number moved since the last draw, or when the ESC row's clock fell due. Nothing else
draws, so an idle front end runs no layout at all.

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
- **Content lives only in the two regions** so that one block is one copy: no third table, no cache
  that answers a second time for the same block.
- **The budget counts content, not positions** so that memory does not follow the length of the
  conversation; a block that leaves comes back through the same ask as one that never arrived.
- **The budget settles against the window the last draw kept** so that a scroll never takes away a
  row it is about to draw; a block outside that window comes back through the same ask as one that
  never arrived.
- **Ingest only appends** so that a batch of stream pieces costs one append each, not one layout
  each; the shape of a round is decided once, at the draw that shows it.
- **One layout per changed position per draw** so that a screen never derives the same block twice
  for one frame.
- **Drawing follows a counter** so that an idle frame costs a comparison, and an animation is the
  only thing that draws without content.
- **Blocks keep ids, not positions**: a rewind changes positions, never ids.
- **A pair is a call id, not a neighbour** because a round commits every request before any result,
  so the call and its answer are not adjacent.
- **`Next` keeps its number** so that a rewind does not renumber anything the reader has already
  seen.
- **The segment is the unit** because coloring depends on the whole run: a fence's syntax state and
  a paragraph's spans cross row boundaries.
- **Gaps belong to the segment's tail** so that laying one segment never has to look at its
  neighbor.
- **Folding lives in the product** because it depends on width, while segments do not.
- **The range is a start plus a height** so that the screen is a slice of the run, and a growing
  block only changes the rows after its last settled row.
- **The pin is a row set after the segments, not a layer above them** so that one scroll model
  covers the checklist and the transcript, and a short screen still shows the checklist.
- **The pin is rebuilt every draw** so that no handover, width change or round close has to know
  about it.
- **Display is stateless** so that the same rows always produce the same screen.
- **A card is drawn from both halves of its exchange**, so the request's position owns the rows; a
  result landing re-lays that position, not only its own.
- **A card is rebuilt from the stream it has**, so partial arguments produce a partial card and
  nothing waits for the call to finish.
- **A compaction round is bracketed, not boxed** so that the fold reads as one place in the
  conversation, the summary below it staying ordinary text.
- **A compaction's figures are asked for, not stored** so that nothing about the gauge has to cross
  the structure the app hands down.

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
| `config.ui.tui.zone.main.history.cacheMargin` | blocks that keep their content and product |
| `config.ui.tui.zone.main.history.focusLine` | rows above the sight line when a block is focused |
| `config.ui.tui.zone.main.history.cardFoldBudget` | rows a card head or tail may take |
| `config.ui.tui.zone.main.history.scrollHeight` | rows one wheel notch walks |
| `config.ui.tui.zone.main.history.todoFold` | task-list folding policy |
| `config.ui.lang` | the language a compaction banner's figures read in |
