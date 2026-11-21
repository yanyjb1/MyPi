# page — the full-screen chassis

The one frame every full-screen page draws inside: a dashed top edge with the title laid over its
middle, the filter's caption and its box, the body, the extra words, a rule, the key hints and the
dashed bottom edge. `resume` and `tree` both fill it; a page never draws its own border.

## 1. Design / Schema

`Panel` is what a page hands over:

| field | meaning |
|---|---|
| `size` | the terminal size the page was started with |
| `title` | the head line, drawn over the middle of the top edge |
| `filter` | `Some` when the page takes a filter word |
| `extra` | lines under the body: the server's opening word, anything said last |
| `empty` | what the page says when the body has nothing |
| `hints` | every key the page answers to, in the order they are worth learning |
| `accent` | the live theme color for the title, the filter's prompt and every key |

A `Filter` is the reader's words, the placeholder shown while it is empty, and a caption sitting
over the box that says what the filter looks at. A `Hint` is the keys to press and what they do.
`draw(body)` returns the whole screen; the page supplies the body lines and nothing else.

### Layout and budget

The order down the screen is fixed: dashed top edge, the caption (with a blank either side when it
earns one), the rule, the filter box, the rule, the body, the `extra` lines, rule, the hint rows,
dashed bottom edge. The reader meets the count first, then one short word on what the filter is for, then the
box, then the data, then whatever the page must say last, and the keys at the very end.

Every rule and both edges are dashed: a dash, a blank, and so on. The title splits the top edge
into two runs of equal length, so the two sides match.

`content()` is the width a page composes its rows at: the terminal's columns less the two border
columns and the two side margins. A body or extra line wider than that is cut at the content
width, so nothing runs over the right margin. `body_rows()` is how many body lines still fit after
everything else is paid for; `page::body_rows` answers the same figure for a caller that has to
size its request before it composes a page.

### Air

The blanks are what a frame earns, never what it assumes. They are granted one block at a time,
bottom first: the keys get their blank rows, then the caption. A block keeps its air only if the
body still holds `FLOOR` rows afterwards; when even the topmost grant would cost the body too much,
the block that asks goes without.

The filter box never takes air: it is the word the reader is typing, and it belongs against the
data underneath it. So a growing terminal spends its new rows on the keys before the caption, and
the data shrinks by two rows per grant, never below the floor. One function decides, and the draw
and the budget both read its answer.

### Margins

The side margin grows with the frame and stops at four columns: `interior / 20`, so a wide terminal
gives the content room to breathe and a narrow one gives it none rather than squeezing it. Every
row sits inside the margin, so a page never computes it.

### Centering

The page's own words — the head line, the filter's placeholder, its caption, the empty word, each
hint row — sit in the middle of the content width. Data rows and the extra lines keep their left
edge: a filter word read back, a list entry, a tree row, a warning.

### Hints

Hints are packed into rows that fit the content width; a hint never splits across two rows, and
each row is centered on its own. The hint rows may take no more than half the room, so a short
terminal keeps a body rather than spending every row on hints; whatever is left at the tail is
dropped, which is why a page puts the keys it cannot do without first. The separator between two
hints on one row is the row's, not a hint's own. A key is painted in the live accent, its
description dim.

## 2. Design intent

- **One frame, many pages.** The edge, the head, the filter box and the hints are the same for
  every page; only the title, the placeholder, the caption, the empty word, the body, the extra
  lines and the hint list differ.

- **No page keeps its own arithmetic.** The row budget, the air, the margins and the clipping are
  one function each, so a page can never miscount and push its own last row off the screen.

- **Words, not colors.** A hint carries text; this module decides that keys are bright and their
  descriptions are dim.

- **Wrapping first, dropping last.** Hints wrap instead of being dropped, so a narrow terminal
  costs the body a row before it costs the reader a key; only a frame too short for both keeps the
  front of the list.

- **Air is bought, not assumed.** A short terminal keeps its data and loses its blanks; a tall one
  shows them. The reader never scrolls a page to look at empty rows.

## 3. Unresolved

1. A page without a filter takes no filter box; there is no mode that hides the box after the
   fact.

2. The chassis owns no scrollbar: a page draws its own window and tells the chassis only what
   fits.

## 4. Config fields

None.
