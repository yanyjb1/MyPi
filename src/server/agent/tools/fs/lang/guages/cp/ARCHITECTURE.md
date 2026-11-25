# cp — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: markers, suffixes, table, relations, graph facts, source flag, parser and checker |
| `scan.rs` | the structural scan: comments and literals blanked out, raw strings kept whole, brackets paired |
| `table.rs` | the line table: declarations, comment runs, preprocessor lines, and the blocks inside a long body |
| `facts.rs` | the graph facts: the project domain, files as units, includes resolved through the compile database, and the database itself |
| `relations.rs` | the relation index: a type's members here and elsewhere, a namespace's declarations, a lone name's x-ray |
| `format.rs` | the parser: one `clang` verdict on a text, and the one line it names when the text does not parse |
| `check.rs` | the checker: the translation unit a file belongs to, `clang-check`'s report, and how a missing program or database degrades |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser and its checker.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts,
  parser and checker.
- The graph facts feed the module graph; the narration is the machine's default.
- `facts.rs` holds the compile database read the parser and the checker share.
- `check.rs` holds the program lookup the parser shares.

## Must stay in step

1. A preprocessor line is blanked before braces are counted, and the compile database is read for its
   include directories alone: neither the table nor the graph ever trusts a macro body to balance.
2. The parser and the checker name the same standard and the same include directories, so a text the
   parser accepts is one the checker can read.
3. A unit is one file, so a header's own declarations are its own exports; nothing here aggregates a
   directory.