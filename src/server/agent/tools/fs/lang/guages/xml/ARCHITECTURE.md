# xml — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: one suffix, the sorter and the parser; no marker, no graph, no relation index, no checker, no tests, no layout |
| `scan.rs` | the tag scan: where each element opens, where it closes, what anchor it answers to, where a tag's `>` is found past a quoted attribute, and how comments, CDATA sections and declarations are stepped over |
| `table.rs` | the line table: one piece per element, its label `element` and the tag or `tag#id` it answers to |
| `format.rs` | the parser: one `quick-xml` pass over a text, the mismatched tag it names, and the open tags it counts at the document's end |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Its parser runs in process, so it asks the shared process gate for nothing and holds no child of its own.
- Answers the registry's contract: the line table, this language's own shape, and a parser. No graph facts, no relation index, no checker, no tests, no layout.

## Must stay in step

1. `scan.rs` is the one place an element's two lines are decided: the table numbers pieces by it, and nothing else reads a tag itself.
2. The line count moves by the newlines of the run just stepped over, never by rescanning the text behind it: a scan of a document is one pass over it.
3. An element's anchor carries its `id` or `name` whenever it has one, so a header a model copies is the header the item mode answers.