# python — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: suffixes, table, kinds, relations, graph facts, source flag, formatter, checker and tests |
| `scan.rs` | the structural scan: comments and strings blanked out, brackets tracked so a folded line is known |
| `table.rs` | the line table: block heads, indentation ends and the blocks inside a long definition |
| `facts.rs` | the graph facts: the project domain, packages as units, their members, exports, edges and forwarding |
| `view.rs` | the module view's narration |
| `relations.rs` | the relation index: class and module members, a lone name's declarations |
| `check.rs` | the checker: the project unit, ty's report, ruff's findings, and how a missing program degrades |
| `format.rs` | the formatter: the wrapper comments around each written block, and where each block landed after `ruff format` laid the text out |
| `test.rs` | the test support: the tests a line range declares, their node paths and one run |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its formatter, checker and test run.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts, formatter, checker and tests.
- The graph facts feed the module graph; the narration is the language's own.
- `check.rs` holds the program lookup the formatter and the test support share.