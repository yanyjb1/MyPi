# lang — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the registry's re-exports |
| `registry.rs` | the registry: the language contract, the language list, the suffix lookup, the workspace's source trees, and the questions a tool asks |
| `graph/` | the graph machine: units, edges, the upward walk, the printing parts and the default narration; it names no language |
| `table.rs` | one file's line table: stamps, paired lines, labels and the queries over them |
| `verdict.rs` | what a formatter, a checker and a test run answer: the draft a formatter is handed, the place a written run lands on, and the verdicts |
| `guages/mod.rs` | declares the languages |
| `guages/rust/` | Rust's declaration and its own knowledge |
| `guages/python/` | Python's declaration and its own knowledge |
| `guages/web/` | the web's declaration and its own knowledge: one sorter per file kind, the page-rooted graph, the three-way anchor index, the biome formatter and checker |
| `guages/markdown.rs` | the Markdown sorter: headings, fences, lists, quotes, HTML blocks, front matter and how far a heading's section reaches |

## Relations

- **`gate`**: the process machine a formatter, a checker or a test run puts its child through; the language supplies the command line and reads the answer.
- **Nothing else in this crate but `summary`** through its consumers: a sorter answers text, never I/O.
- **Who calls it**: `read` (line numbers, budget, graph facts, source tree), `splice` (receipt anchors, the formatter, the check), `edit`, `fault` (the check and the tests), `change`, `skeleton`, `grep` (its scan, its outline, its relation sections), `write` (the formatter, the check).
- **Knows nothing about**: the workspace, the compiler's own vocabulary, any tool's caps or prose.

## Must stay in step

1. The opening and closing lines of a piece pair; a closing line names the line it closes. A tool that reads spans and a tool that reads reaches must agree on that pairing.
2. A label is the head's template with the names cut out; an exact is the head's own text. A tool that judges what a piece is reads the language's shape; a tool that asks where a name is declared reads the relation index. Neither cuts a label itself.
3. A new language is one knowledge file and one registry entry; nothing else in this crate learns its name or its suffixes.
4. A diagnostic reaches the write it belongs to by the name the checker gives that file, and by the line the checker reports; both must agree.
5. A file with no sorter numbers every line, so a new sorter narrows what a tool shows and never widens it.
6. A test's reach — its first attribute to its closing line — is derived for the test alone and never written back into the table, so no other reader sees it.
7. A sorter is handed the file's path with its text: a language covering several file kinds chooses its own grammar by the name, and a single-kind language ignores it.