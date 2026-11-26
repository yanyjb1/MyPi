# toml — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: one suffix, the sorter and the parser; no marker, no graph, no relation index, no checker, no tests, no layout |
| `scan.rs` | the table scan: which line each header opens on, which line its last entry stands on, the name it carries, and the multi-line strings it steps over |
| `table.rs` | the line table: one piece per table, its label `table` and the name it answers to |
| `format.rs` | the parser: one whole-document verdict from the `toml` crate, and the line its byte span falls on |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Its parser runs in process, so it asks the shared process gate for nothing and holds no child of its own.
- Answers the registry's contract: the line table, this language's own shape, and a parser. No graph facts, no relation index, no checker, no tests, no layout.

## Must stay in step

1. `scan.rs` is the one place a table's two lines are decided: the table numbers pieces by it, and nothing else reads a header itself.
2. A header closes the table above it and opens its own; the leading keys are one piece of their own, whether a header follows them or not.
3. A blank line belongs to no table, so a table's piece ends on its last entry and never on the trailing gap that follows it.
4. A line inside a multi-line string is stepped over whole: it opens no table, and no entry stands there.