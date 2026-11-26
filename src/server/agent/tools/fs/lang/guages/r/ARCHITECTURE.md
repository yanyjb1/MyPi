# r — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: suffixes, table, kinds, relations, source flag, parser and tests; no markers, no graph, no checker |
| `scan.rs` | the structural scan: the byte order mark dropped, comments and strings blanked, a backtick name kept as code, braces and parens paired per line |
| `table.rs` | the line table: the file's own level, its bindings and calls, each piece's span and own text |
| `relations.rs` | the relation index: a lone name's declarations at the file's own level |
| `format.rs` | the parser: one `air` verdict on a text, the line it names, the program lookup the tests share |
| `test.rs` | the test support: the test file a change asks for, one `Rscript` run, its blocks counted and its failures placed |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser and its test run.
- Answers the registry's contract: the line table, the language's own shape, relations, the parser and the tests.
- `format.rs` holds the program lookup `test.rs` shares.
- It names no graph facts, so `read` shows no module view for its files and a directory of R files is no source tree of its own.

## Must stay in step

1. A piece's label ends in the kind `shape` answers to, and the name before it is what `relations` matches. One side reading a label the other does not write finds nothing.
2. The parser and the test run find their programs the same way: beside this executable, then on `PATH`, with the extension Windows keeps in a program's own name.
3. A derived test path is asked for once, however many spellings of the suffix reach the same file.
4. The driver's `LOADFAIL` line is the only word a project that will not load sends; `test.rs` reads it before it counts any block.
