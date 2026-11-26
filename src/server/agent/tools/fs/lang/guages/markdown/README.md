# markdown

A note is a unit, and the notes it names are what hangs under it.

## 1. Design / Schema

- Suffixes: `.md`, `.markdown`.
- A directory that holds a note outright and names no project of another language is a tree of its own; the root a note's names are searched from is the repository it sits in, the nearest directory above it holding `.git`, or its own directory when there is none.
- A unit is one note. Its name is its stem — the word another note calls it by.
- The line table reads front matter, headings, fences, lists, quotes and HTML blocks; a heading's reach stops before the next heading of the same or a shallower level, trailing blank lines aside.
- Three references are read, always outside a fenced block:
  - `[[name]]`, `[[name|alias]]`, `[[name#Heading]]`, and the embedded form: a bare note name, searched by stem across the repository. Every note carrying that stem is a landing file of its own, so an ambiguous name shows every note it may mean.
  - `[text](path)`: a path relative to the note's own directory, with `.md` understood when the text leaves it off. A target with a title, a scheme other than none, or a space names nothing.
  - `` `path/to/file.ext` ``: a file name in a code span, kept only when it holds no space, opens no `\`, and its suffix is one a language of this build answers to. The file must exist.
- A reference that lands nowhere keeps the text it wrote, so a broken link still shows with `(missing)`.
- An anchor names a heading on the landing note: the row prints it with the line that heading opens on.
- Graph facts: one note per unit, its references as edges, the notes it names as its territory (`note` or `file`), its headings as its interface, the repository as its domain, and a directory holding notes as its loose tree.
- Narration: the headings this note writes, the notes it gathers with their length and own headings, what it links out to, and which notes link back to it.

## 2. Design intent

- **A note names a note, not a path.** The bare name is what an author writes and what a reader remembers, so the search is by stem, and a shared stem answers with every candidate rather than a silent first pick.
- **Links are a graph, not a check.** A broken link shows as `(missing)` and never faults a write: a note is written before the note it points at exists.
- **A file in a code span is a pointer.** Prose that writes `` `check.rs` `` is pointing at that file, but only a suffix this build knows makes it one; `cargo check`, `Option<T>` and `foo.bar()` name nothing.
- **Reading a table of contents reads the whole shelf.** `down` prints each named note's length and headings, so one read of an index shows what it gathers without a read per note.
- **No formatter, no checker, no tests.** Prose has no layout a tool should impose, and the structure is already the answer.

## 3. Unresolved

- Front matter's `tags` and `aliases` are read as lines, not as names this language exposes.

## 4. Config fields

None. Markdown reads no key of its own.