# rust — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: suffixes, table, kinds, relations, graph facts, source flag, parser, checker and tests |
| `table.rs` | the line table: head words, key-line passes and head templates |
| `scan.rs` | the structural scan: comments and literals cut out, braces paired and closing lines found |
| `relations.rs` | the relation index: type and module members, a lone name's declarations and impl blocks |
| `facts.rs` | the graph facts: units, declarations, exports, edges and the source domain |
| `view.rs` | the module view's narration |
| `test.rs` | the test support: the tests a line range declares, their full paths and one run |
| `check.rs` | the checker: the build unit, its diagnostics and their placement |
| `format.rs` | the parser: the edition from the nearest manifest, one `rustfmt` verdict on a text, and the one line it names when the text does not parse |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser, checker and test run.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts, parser, checker and tests.
- The graph facts feed the module graph; the narration is the language's own.