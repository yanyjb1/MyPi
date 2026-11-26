# lang — design

One language, one file; every file tool reads the same table and asks the same questions.

## 1. Design

A language answers one line table for one file and declares what else it can do. Every sorted language is one file under `guages/`, and a language covering several file kinds — the web's pages, stylesheets and scripts — is handed the file's own path so its sorter reads the kind from the name.

**The table.** For every line: what that line is, the line it pairs with, the label its head reads by, and the head's own text. An opening line and its closing line pair into one piece.

**The declarations.** A language says what a tool may ask it:

- the files that mark one of its projects in a directory, and the suffixes it answers to;
- whether its pieces can index relations, and how;
- whether it knows its units and edges: the unit a file belongs to, the units a unit declares, what a file exposes, where to look for importers, the tree a directory owns when it holds no marker, and how to narrate the result;
- whether its files are source, which decides how much of one a read shows;
- whether it has a parser, and how to ask it whether a text parses; the parser judges and never lays a text out;
- whether it has a checker, and how to find and run the unit a file belongs to;
- whether it has tests, how to find the tests a change wrote, and how to run those alone.

A **stamp** is one of four: open, close, single, plain. A **label** is the head's template with the names cut out (`fn`, `impl a for b`); an **exact** is the head's own text (`impl Display for Env`, `## Design intent`). A consumer never reparses a header.

A piece's last line may be a key line of its own — Rust's `}`, a fence's second line, a list's last line — or ordinary prose, which carries no number. A sorter says which by how it writes the piece.

Suffixes point at languages. A file whose suffix meets none is a table where every line is its own piece, and no declaration is there to read.

## 2. Design intent

**One answer, many readers.** `read` numbers lines by it, `splice` anchors its receipt on it, `edit` and `change` find the piece around a change, `fault` draws its seam, `grep` builds its outline and its relation sections.

**One language, one file.** Every fact about a language — its structure, its parser, its checker — lives in that language's own file. A tool asks the declarations; it never names a language, a suffix or a toolchain.

**One language may own several file kinds.** The web is one language over pages, stylesheets and scripts together, because a DOM anchor is only worth looking up when one query reaches a page, a stylesheet and a script at once. Its sorter dispatches on the suffix it is handed.

**A language declares as much as it can answer for.** The notebook answers a table and a toolchain and nothing else: it has no graph of its own, so it carries no unit, no edges and no territory, and a directory of notebooks is no source tree. A language that declares less narrows what a tool asks it, never what a tool shows.

**Every language in scope answers its own part.** A relation query is asked of each language the batch holds, over that language's own files. One language alone keeps its own way of saying nothing was found; several together print only those that found something, so a workspace-wide search never carries a list of "not here" from every other language.

**The checker knows its own unit.** A build unit is whatever the language says it is: a crate, a module, a package, a page root. The shared side groups the batch's writes by unit, runs the language's checker once per unit after the last write, and hangs every diagnostic under the edit whose lines it hit. A language with no checker takes no part.

**A batch tests only what it wrote.** The language names the tests a line range declares; the shared side asks the checker's own list for their full paths and runs those alone, and a name that list does not carry is reported, never counted as a pass.

**The process machine belongs to no language.** Starting a child, its deadline, its tree and the turn's stop are the gate's; a language supplies the command line and reads what came back.

**Structure, not layout.** A sorter says what the pieces are and where they end. It never says how they should look: clipping, indentation, folding and prose stay in the tool that shows them.

## 3. Unresolved

- A heading's section is remembered by its heading line, so its end is a line no tool may name.
- A language is chosen by suffix alone: two languages sharing one suffix cannot be told apart.
- A language's parser reads one text, so a file format whose structure is not its own text — a notebook, whose cells are Python written inside JSON — has a position that its own scan must translate back.
- A diagnostic reaches the write it belongs to by the name its checker gives that file; a checker that names files unlike its own cannot be placed.
- A test's full path comes from the toolchain's own list, so a name two modules share runs in both.

## 4. Config fields

None. The registry reads no key of its own.