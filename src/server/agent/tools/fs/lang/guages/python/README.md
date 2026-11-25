# python

Python's own declarations, registered in `mod.rs`: its suffixes, line table, graph facts, parser, checker, tests and relation index.

## 1. Design / Schema

- Suffixes: `.py`, `.pyi`.
- A project is found by `pyproject.toml`, `setup.cfg`, `setup.py`, `requirements.txt` or `.git`, whichever is nearest; its modules live under `src/` when that holds a package, the project root otherwise.
- A unit is a package directory (one holding `__init__.py`) or a single module file. An `__init__.py` belongs to its package, so reading it names that package.
- The line table reads definitions, imports, the script's own guard, decorator runs and comments. A block ends where the line that opened it is reached again at its own indentation.
- Graph facts: the unit a file belongs to, the units a package holds, exports, edges and the source domain.
- Exports: a written-out `__all__` when one exists, the public top-level definitions and constants otherwise.
- Edges: top-level imports and the ones an `if TYPE_CHECKING:` guard holds, resolved against the file's own package. A name forwarded through `__init__.py` files is followed up to three hops.
- Parser: `ruff format`, read for its verdict alone: what it lays out is thrown away, so a written file keeps the lines the model sent.
- Checker: `ty` types the whole project, `ruff check` lints the files a batch wrote.
- Tests: `pytest`, each test asked for by its own node path.
- Relations: `A.b` as a class member or a module member, and a lone name's declarations.

### Relations output

The relation index prints three sections, in this order:

- `[class] A.b` — a class member, or why no class `A` declares `b`;
- `[module] A.b` — a module member, or why no module `A` holds `b`;
- `[x-ray] A` — the declarations of `A` and the members of a class it opens.

At most 12 members are listed per block.

## 2. Design intent

- Python declares nothing about which package holds it: the directory says it, so the graph reads directories.
- Imports are read statically, as written. A name reached through `importlib`, a local import or a fallback `try` block is not followed: the graph is a signpost, not a runtime.
- A finding `ruff` raises is a warning, never an error: a linter that called its own findings fatal would stop a batch that only needs to hear them.
- Which relations live here, and how many members a row lists, are this language's own limits.

## 3. Unresolved

- A `__all__` built by concatenation or a call falls back to the public-name rule.
- A directory without `__init__.py` is not a package, so a namespace package reads as loose modules.
- `pytest` must be on PATH; a project that keeps it in a virtual environment is not run.

## 4. Config fields

None. Python reads no key of its own; the toolchain is found beside this executable or on PATH.