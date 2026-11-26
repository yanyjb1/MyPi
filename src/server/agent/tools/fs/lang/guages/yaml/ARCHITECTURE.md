# yaml — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: two suffixes, the sorter and the parser; no marker, no graph, no relation index, no checker, no tests, no layout |
| `scan.rs` | the block scan: how a line reads, what a key opens below it, how far a nested body reaches, how far a scalar's body runs, and which text is a scalar's marker |
| `table.rs` | the line table: one piece per key or item, and the scalar bodies it steps over whole |
| `format.rs` | the parser: one `saphyr` verdict over every document of a text, and the line it names when the text does not parse |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Its parser runs in process, so it asks the shared process gate for nothing and holds no child of its own.
- Answers the registry's contract: the line table, this language's own shape, and a parser. No graph facts, no relation index, no checker, no tests, no layout.

## Must stay in step

1. `scan.rs` is the one place a line's indentation and reach are decided: the table numbers pieces by it, and nothing else measures indentation itself.
2. A scalar's body is measured from its marker line's own depth, so a sibling item or key at that depth ends the body and starts a piece of its own.
3. A line inside a scalar is stepped over whole: it carries no number, and nothing that follows is read from it.