# go — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: suffixes, table, kinds, relations, graph facts, source flag, parser, checker and tests |
| `table.rs` | the line table: head words, grouped declarations, comment blocks, block heads inside a long function, and Go's own delimiters |
| `scan.rs` | the structural scan: comments and literals cut out, braces and parens paired |
| `relations.rs` | the relation index: a package's declarations, a type's methods and fields, and a lone name's own declarations |
| `facts.rs` | the graph facts: packages as units, the module domain, imported packages, exports and import reading |
| `check.rs` | the checker: the module unit, `go build` per written package, `go vet` on what compiled, and the two reported streams |
| `format.rs` | the parser: one `gofmt -e` verdict on a text, and the line it names when the text does not parse |
| `test.rs` | the test support: the `Test*` functions a range declares, their listing and one run |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser, checker and test run.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts, parser, checker and tests.
- The narration is the machine's default; this language supplies no view of its own.
- The graph facts feed the module graph; `check` and `test` share the module unit and the path naming a fault hangs by.