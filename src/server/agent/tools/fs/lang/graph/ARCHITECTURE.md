# graph — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the entries' re-exports |
| `types.rs` | the shared shapes: units, declarations, edges, children and one neighbourhood |
| `entry.rs` | the doors: the unit a file belongs to, its neighbourhood and the rows a view reads |
| `loader.rs` | the one read-only disk door: a per-call cache, filtered directory entries, the text cap, and the per-root name index a language naming a file by its stem reads |
| `index.rs` | turning facts into a neighbourhood: unit lookup, child expansion, size, incoming/outgoing and declared-by |
| `walk.rs` | the upward search: the deadline, whole-level drops and the boundary |
| `parts.rs` | the printing parts: relative paths, plurals, clipping, headings, node/edge/boundary lines and grouped importers |
| `view.rs` | the default narration, for a language with no view of its own |

## What it talks to

- **The language's module facts**: as callbacks. This layer never names a language.
- **The language's suffixes**: the suffixes a walk may read, so foreign files are never parsed.
- **The row shape every narration answers.**
- **`read`**: the only consumer today; it decides when to build, keeps the per-unit primer memory and stamps the file it read.
- **Who calls it**: `read` through the language's module facts.

## Must stay in step

1. The boundary's fields and a language's scan line: the fields a view needs to say why a walk ended must stay in one answer.
2. A unit and the fine file it lands on: incoming matching uses the unit, child dependencies use the file. Collapsing one into the other loses either the unit count or the sibling names.
3. The walk's suffix filter and the registry's suffixes: a language that answers for a pointer but is not in the language list would walk every file.