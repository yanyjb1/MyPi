# json — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: one suffix, the sorter and the parser; no marker, no graph, no relation index, no checker, no tests, no layout |
| `scan.rs` | the bracket scan: where each container opens, where it closes, and the key that introduces it |
| `table.rs` | the line table: one piece per container, its label `container` and the key it answers to |
| `format.rs` | the parser: one `serde_json` verdict on a text, and the line and column it names when the text does not parse |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Its parser runs in process, so it asks the shared process gate for nothing and holds no child of its own.
- Answers the registry's contract: the line table, this language's own shape, and a parser. No graph facts, no relation index, no checker, no tests, no layout.

## Must stay in step

1. `scan.rs` is the one place a container's two lines are decided: the table numbers pieces by it, and nothing else reads a bracket itself.
2. The scanner and the parser agree by construction: a document whose brackets do not pair answers no pieces, and that is exactly a document the parser refuses.
3. A container's piece never names a line between its own two ends, so a fold shows the document's shape and nothing of its content.