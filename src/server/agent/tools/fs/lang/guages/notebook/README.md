# notebook

A `.ipynb` file is located, not modelled: each of its cells is one piece a tool can name.

## 1. Design / Schema

- Suffixes: `.ipynb`.
- A cell is a piece. Its label is its own kind — `code`, `markdown`, `raw` — and its exact is `cell N (kind)`, where `N` counts the `cells` array from 1, the number ruff uses.
- A cell's span runs from the line opening its `{` to the line closing it, so `source` and `outputs` are both inside it and neither is a piece of its own. The lines before the array and after it carry no number.
- The cells are read by counting brackets and stepping over quoted text, so a cell that writes its whole body on one line is one piece on that line. A key is only a key where its quotes stand unescaped and a colon follows, so a cell's own output text naming `source` or `cell_type` is not mistaken for one.
- Parser: `ruff format` over stdin, read for its verdict alone. ruff names a notebook position `path:cell:row:column`; the cell's own source rows turn that row into the file's line, and the sentence says which cell it fell in.
- Checker: `ruff check` over the files a batch wrote. A finding's own `cell` field becomes the file line the same way, and the message is prefixed with `cell N (kind)`. A cell that does not parse is an error; every other finding is a warning. A finding whose cell the scan cannot name keeps the row ruff gave it, so a scan that came up short loses no diagnostic.
- No graph facts, no relations, no tests, and no project marker: a directory of notebooks is no source tree.

## 2. Design intent

- **A cell is where a model thinks.** A notebook is read to answer "where does this live", and the answer is a cell, not a line. The piece boundaries are the `{` and `}` a cell occupies, so a piece's span is the whole of what that cell holds.
- **The cell number is ruff's number.** Two tools name cells, the sorter and the checker; both count the array from 1, so a fault, an item header and an answered span point at one cell.
- **Locating, not modelling.** A notebook declares no imports a graph could follow and runs no module of its own, so it carries no unit, no edges and no interface. What it does carry is a place.
- **A row is not a line.** ruff counts rows of Python, and a source written as one string carries a whole cell's rows on the one line it stands on. The scan counts each string's own `\n` escapes, so a fault lands on the line a reader can find.
- **A payload folds, a line number does not.** An embedded image is thousands of lines of base64; the mask folds that run into one line and spends the rest, so every line keeps its own number and a fold shows as a gap. The line that stands for a run carries its first line's place, and every report that draws a line from a file rides that place rather than a running count, so a change inside a fold is answered by the one line that speaks for it.

## 3. Unresolved

- A notebook whose JSON is laid out on one line is one piece holding the whole file; a cell cannot be named inside it.
- The parser reads `source` cells alone, so a `raw` cell whose bytes are not UTF-8 is reported against the notebook rather than the cell.
- A notebook whose `source` is written as one string on one line has every one of its rows standing on that same line, so two faults in one cell name the same place.
- `ruff` must be on PATH; a project that keeps it in a virtual environment is not run.

## 4. Config fields

None. Notebook reads no key of its own; the toolchain is found beside this executable or on PATH.