# markdown — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: suffixes, table, the note graph, the source flag; no marker, parser, checker or tests |
| `table.rs` | the line table: front matter, headings and their reach, fences, lists, quotes and HTML blocks |
| `links.rs` | the reference scan: wikilinks, relative links, file names in code spans, and where each one lands |
| `facts.rs` | the graph facts: notes as units, the repository as the domain, a note-holding directory as a loose tree, references as edges, named notes as territory, headings as the interface |
| `view.rs` | the note's narration: headings, the notes it gathers, its links out, its links back |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Answers the registry's contract: the line table, this language's own shape, and the note graph.
- The graph facts feed the module graph; the narration is this language's own.
- Reads file names through the shared loader's name index, which walks a root once per loader.

## Must stay in step

1. `links.rs` and the sorter agree on what a fenced block is: a reference inside one is never read, so both use `table.rs`'s fence rules.
2. A reference's landing file and the loader's name index: a name is a stem, and every stem match is its own edge, never one edge with several targets.