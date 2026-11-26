# toml

A `.toml` document: one piece per table, and the toml crate is the only judge of whether it reads.

## 1. Design / Schema

- Suffix: `.toml`.
- A table is a piece, from its `[header]` line to its last entry. `[[array.of.tables]]` is a table like any other. A table's exact is its name without brackets; its label is `table`.
- The document's own leading keys, standing before any header, are one piece whose exact is absent: no header introduces them. A document with no header at all is that one piece.
- A header closes the table above it. A blank line does not: a table ends on its last entry, so a run of blank lines below a table belongs to nobody.
- A line inside a multi-line string (`"""` or `'''`) is that string's text, so a `[a]` there opens no table.
- Parser: the `toml` crate in process, reading the whole document. It reports a byte span, and the line is counted up to that span's start.
- No layout, no checker, no tests, no graph, no relations, no project marker: a commit leaves the file exactly as the model wrote it.

## 2. Design intent

- **A table is what a model navigates by.** A TOML document is a set of named tables, and a question about one is answered by its header's line. Numbering every key would bury that table under its own entries.
- **Not reformatted, ever.** A formatter that answers TOML in the general case rewrites quoting style and drops comments. A commit that touched a config file's formatting would be a change the model never asked for.
- **Duplicates are errors.** The crate refuses a key written twice in one table, which is the one mistyping a hand-edited config most often carries.

## 3. Unresolved

- A multi-line array or a multi-line string inside a table is not a piece of its own; its continuation lines carry no number, and an entry that follows it is still read as an entry because it holds a `=`.
- A table written entirely on one line — `[a] b = 1` — numbers its header and its entry on the same line.

## 4. Config fields

None. TOML reads no key of its own.