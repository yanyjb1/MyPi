# page — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | crate-visible |
| `chassis.rs` | `Panel`, `Hint`, `Filter`, the layout and its row budget, the margins, the clipping, centering, hint packing | crate-visible |
| `picker.rs` | `Picker`: one column of rows, the indices the filter keeps, the cursor, the window | crate-visible |

## What it exposes

- `Panel` with `margin`, `content`, `body_rows` and `draw`; `divider` for a page that draws its own seam.
- `Hint`, `Filter`, and the free `body_rows` for a caller that needs the figure early.
- `Picker` with `replace`, `edit_filter`, `move_by`, `first`, `last`, `select`, `settle_scroll`,
  `row_at`, `row`, `position`, `clear_rows`, `clear`, and the getters a renderer reads.

## What it calls

- `zone::TermSize` for the size the page was started with.
- `text::card` for the dashed edge, the labeled edge, the row and the clip; `theme::fixed` for the
  colors, `theme::Accent` for the live one.
- `unicode_width` for display columns.

`picker` calls nothing outside the standard library: a row's matchable text arrives as a closure,
so it never names a row type's fields.

## Who calls it

`resume`, `tree` and `hub`, each once from its renderer. `main` does not: the chat area draws its
own border, which is the transcript's, not a page's.

`Picker` is a field of `resume`, `tree` and `hub`: each hands rows in from its own side effects,
reads it while drawing, and writes the window back after the draw. No other zone lists rows.

## What it does not know

What any page says, what its body holds, which keys it answers, or who is reading. A page hands
over finished lines; the chassis places them and cuts them to its width.

The picker knows less still: not a row's fields, not what a key means, not a color. A page says
which text a row matches on and calls one call per key; the picker only keeps the column consistent.

## Must stay in sync

1. `FRAME`, `RULE`, `CAPTION`, `BOX`, `AIR` and the rows `draw` emits: one more row of any is one
   edit in both, and in `air`. `CAPTION` and `BOX` are the rows the two blocks take with no air;
   a note is one more when the page has one; `air` adds the blanks around the note, the caption
   and the keys, granting them in that order from the bottom up, and never around the box.
2. `air` and `draw`: the grants the chassis draws and the grants its budget assumed are the same
   answer, from one function.
3. `body_rows` and `draw`: the budget a page sizes itself by is the budget the body is given.
4. `Hint`'s width and the separator: a hint's own width never includes the row's separator.
5. `content` and what a page draws: a page composes its data rows at `content`, never at the raw
   terminal width; `fitted` is what keeps a longer one inside.
6. `visible` and every call that changes the rows: the indices point into `rows`, so a page reads a
   row with `row_at` and never indexes the list itself.
7. `selected` and `scroll` and every call that can move them: both stay inside the visible set, so
   a renderer may read them without checking.

## Not wired

1. No scrollbar, no cursor: a page owns its window and its position.
2. No alignment setting; the chassis takes one placement for words and one for data.
3. No page count and no per-page height: the picker moves the cursor by rows, and a page that lists
   taller blocks computes its own window.
