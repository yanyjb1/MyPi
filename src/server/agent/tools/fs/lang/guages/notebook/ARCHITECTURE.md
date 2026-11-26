# notebook — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: one suffix, the sorter, the parser and the checker; no marker, no graph, no relation index, no tests |
| `scan.rs` | the cell scan: each cell's kind, the lines its brackets open and close on, and the line its `source` starts on |
| `table.rs` | the line table: one piece per cell, its label its kind and its exact `cell N (kind)` |
| `format.rs` | the parser: one `ruff` verdict on a text, and the cell and file line it names when the text does not parse |
| `check.rs` | the checker: the notebook's own directory as its unit, ruff's findings, and how a cell row becomes a file line |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser and checker, and the shared verdict shapes for their answers.
- Answers the registry's contract: the line table, this language's own shape, a parser and a checker. No graph facts, no relation index, no tests.
- Names no other language: the program lookup is its own copy, so no language learns another's name.

## Must stay in step

1. `scan.rs` is the one place a cell's lines are decided: the table numbers pieces by it, and the checker turns ruff's cell rows into file lines by it, so both agree on where a cell starts and what its `source` line is.
2. A cell counts from 1 both here and in ruff's report; a change to that counting breaks the checker's placement and the item headers at once.
3. The parser and the checker name a cell the same way, `cell N (kind)`, so a model reading one can find the other.