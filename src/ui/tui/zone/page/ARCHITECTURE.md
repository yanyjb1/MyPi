# page — architecture

What each file holds and who calls it. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | crate-visible |
| `chassis.rs` | `Panel`, `Hint`, `Filter`, the row budget, the margins, centering, hint packing | crate-visible |

## What it exposes

- `Panel` with `margin`, `content`, `body_rows` and `draw`.
- `Hint`, `Filter`, and the free `body_rows` for a caller that needs the figure early.

## What it calls

- `zone::TermSize` for the size the page was started with.
- `text::card` for the edge and the row; `theme::fixed` for the colors, `theme::Accent` for the
  live one.
- `unicode_width` for display columns.

## Who calls it

`resume` and `tree`, each once from its renderer. `main` does not: the chat area draws its own
border, which is the transcript's, not a page's.

## What it does not know

What any page says, what its body holds, which keys it answers, or who is reading. A page hands
over finished lines; the chassis places them.

## Must stay in sync

1. `FIXED` and `BOX` with the rows `draw` emits: one more row of either is one edit in both, and in
   the budget.
2. `body_rows` and `draw`: the budget a page sizes itself by is the budget the body is given.
3. `Hint`'s width and the separator: a hint's own width never includes the row's separator.
4. `content` and what a page draws: a page composes its data rows at `content`, never at the raw
   terminal width.

## Not wired

1. No scrollbar, no cursor: a page owns its window and its position.
2. No alignment setting; the chassis takes one placement for words and one for data.