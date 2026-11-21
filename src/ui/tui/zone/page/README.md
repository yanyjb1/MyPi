# page — the full-screen chassis

The one frame every full-screen page draws inside: a dashed top edge with the title laid over its
middle, the filter's caption and its box, the body, the extra words, a rule, the key hints and the
dashed bottom edge. `resume`, `tree` and `hub` fill it; a page never draws its own border.

Beside the chassis sits `Picker`, the column of rows those two pages share: the rows, the words that
filter them, the cursor and the window. The chassis places finished lines; the picker holds what a
page lists.

## 1. Design / Schema

`Panel` is what a page hands over:

| field | meaning |
|---|---|
| `size` | the terminal size the page was started with |
| `title` | the head line, drawn over the middle of the top edge |
| `filter` | `Some` when the page takes a filter word |
| `note` | one sentence on what the page is for, under the head |
| `extra` | lines under the body: the server's opening word, anything said last |
| `empty` | what the page says when the body has nothing |
| `hints` | every key the page answers to, in the order they are worth learning |
| `accent` | the live theme color for the title, the filter's prompt and every key |

A `Filter` is the reader's words, the placeholder shown while it is empty, and a caption sitting
over the box that says what the filter looks at. A page without a filter puts the same sentence in
`note` instead, so every full-screen page opens with one line saying what it holds. A `Hint` is the keys to press and what they do.
`draw(body)` returns the whole screen; the page supplies the body lines and nothing else.
`divider(width)` is the same rule, offered to a page that cuts its own body into two bands.

### Layout and budget

The order down the screen is fixed: dashed top edge, the note (with a blank either side when it
earns one), the caption over the box (with a blank either side when it earns one), the rule, the
filter box, the rule, the body, the `extra` lines, rule, the hint rows, dashed bottom edge. The
reader meets the count first, then one short word on what the page holds, then the box, then the
data, then whatever the page must say last, and the keys at the very end.

A note and a caption meet on one blank, never two: the blank belongs to the seam, so air granted to
either block opens that seam without doubling it. A page with a note and no filter box closes the
note's band with a rule, so the head keeps its own band instead of bleeding into the data.

Every rule and both edges are dashed: a dash, a blank, and so on. The title splits the top edge
into two runs of equal length, so the two sides match.

`content()` is the width a page composes its rows at: the terminal's columns less the two border
columns and the two side margins. A body or extra line wider than that is cut at the content
width, so nothing runs over the right margin. `body_rows()` is how many body lines still fit after
everything else is paid for; `page::body_rows` answers the same figure for a caller that has to
size its request before it composes a page.

### Air

The blanks are what a frame earns, never what it assumes. They are granted one block at a time,
bottom first: the keys get their blank rows, then the caption, then the note. A block keeps its air
only if the body still holds `FLOOR` rows afterwards; when even the last grant would cost the body
too much, the block that asks goes without.

The filter box never takes air: it is the word the reader is typing, and it belongs against the
data underneath it. So a growing terminal spends its new rows on the keys before the caption and
the note, and the data shrinks by two rows per grant, never below the floor. One function decides, and the draw
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

- **A seam is one blank.** Two blocks that meet share the blank between them instead of each
  opening one, so a page carrying both a note and a caption never doubles the gap.

## 3. Picker

A page that lists rows keeps a `Picker<Row>` instead of its own copy of the column. It holds the
rows, the indices that passed the filter, the cursor and the first drawn row, and nothing else: not
the keys, not the colors, not what a row means.

| call | what it does |
|---|---|
| `replace(rows, haystack)` | a whole new list; the visible set is rebuilt, the window stays |
| `edit_filter(edit, haystack)` | the reader edits the words through `edit`; the visible set follows, and the window returns to the top |
| `move_by` / `first` / `last` / `select` | the cursor, never past either end |
| `settle_scroll(body)` | the window, kept on the cursor |
| `row_at` / `row` / `position` | a row by screen position, the row under the cursor, the position of a row that matches |
| `clear_rows` / `clear` | no list arrived / the page was left |
| `filter` / `visible_len` / `selected` / `scroll` / `set_scroll` | what the renderer reads and writes back |

`haystack` is the page's own closure: it says what text a row can be matched on, in which language.
The skeleton lowercases the words and matches them; it never learns a row's fields.

### Why the skeleton keeps no keys

A page decides which key means what, and calls one call here. A new key on a new page costs one
line in that page's own key match and nothing here. A skeleton that read keys would have to be
reopened every time a page grew one, and would then be a name for a shape rather than a shared rule.

### The two invariants

The visible set holds indices into the rows, never copies, so a page can address a row by its
screen position without the list being reordered. The cursor and the window are both kept inside
the visible set by every call that can move them; a page writes `set_scroll` back after its draw
only to record where it actually started drawing.

## 4. Design intent (picker)

- **One column, one owner.** Filtering, clamping the cursor and keeping the window on it are three
  rules, and a page that restates them will eventually restate them differently.

- **Replacing the list is one call.** A page whose rows are rebuilt from live data (the tree) and a
  page whose rows arrive whole (the session list) both say `replace`; neither tracks a row count to
  decide when to recompute.

- **The window is not reset by a refresh.** Only the reader's own edits send it back to the top. A
  refresh keeps it, so a list that grows underneath the reader does not jump.

## 5. Unresolved

1. A page without a filter takes no filter box; there is no mode that hides the box after the fact.
2. Neither the chassis nor the picker draws a scrollbar; a page owns its window and tells them only
   what fits.

## 6. Config fields

None.
