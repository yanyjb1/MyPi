# read

One path per item: a file with its line numbers, a directory with its entries, a Rust file with its
module's context. Every answer is a header line, a body, and what was left out at its end; the tool
bounds its own answer, which is why the front door never measures it.

## 1. Design / Schema

### One call

`files` is one item per path, each with its own `offset` (1-based, default 1), `limit` and `item`.
Items are answered in the order the call names them. One item's failure is that item's: it answers a
header, no body, and its error last, exactly like any other answer, and the ones after it still run.

A file's budget comes from its kind; `limit` narrows the line count and never widens it.

| kind | lines | bytes |
|---|---|---|
| a source file | 800 | 256 KiB |
| anything else | 200 | 64 KiB |

### The three parts

Every answer opens with `==> path`; a directory adds its own counts to that line. The body follows,
and the footer holds what the reader was not given — the window's status line, an error, or the line
an item search owes when it found nothing. Nothing is printed before the header and nothing after the
footer. A lone item names its path as a batch does: the header never depends on how many paths the
call named.

### A file's body

A body carries line numbers only where a change can start or end. `splice` locates a change by line,
so a session with `splice` numbers a file whose suffix has a sorter at each key line its table
stamps: a declaration, a
run of `use` statements, a hat of attributes and comments (its first and its last line, however many
it stacks; the file's own head counts even when a blank line follows it), the closing brace of any
numbered block, and — inside a function longer than fifteen lines — every block head: `if`, `else`,
`match`, `while`, `for`, `loop`, `unsafe` and `async`, whether it stands on its own, follows a `}`,
or opens behind `let x =`, an assignment, `return` or `break`. A `let … else` counts, and so does a
macro call or a closure whose `{` ends the line. No head needs its `{` on that line: a folded
condition puts it three lines down. Every other line stays unnumbered, so every numbered block has a
`(start, end)` a call can name without counting. A file no sorter takes is one whole piece per line:
every line carries a number.
A Markdown file numbers what its grammar makes structural: every heading line, the two lines of a
fenced block, a list's first and last line (and, when items nest, each outer item's own line), a
quote's first and last line, an HTML block's two tag lines, and the front matter's pair of `---`.

`edit` locates a change by text, so a session with `edit` and without `splice` gets no numbers at
all: the body is the file, ready to be copied into `old`.

The gutter is as wide as the file's own last line number plus one pipe, so a three-digit file writes
`123|` and an unnumbered line writes `   |`: the pipe keeps the body in one column and makes the
boundary between gutter and code visible. No minimum width is imposed; a seven-line file writes
`7|`. An empty line is a newline and nothing else. The gutter's bytes are part of the body's budget.

The footer carries the status line when the file continues: `[800 of 900 lines; next offset 801]`, or
`[256 KiB cap; 161 of 200 lines; next offset 162]` when bytes ran out first. The numbers are the
file's own budget, 800/256 KiB for a source file and 200/64 KiB for anything else. A line longer than
the room left is cut at a character boundary and not counted, so the next read starts at that same
line. A file that ended, and a window that starts past its end, answer no status line at all.

A file whose head names a known image format (PNG, JPEG, GIF, WEBP; the bytes decide, never the
name) answers no text at all: its bytes go as an image part and nothing else. The gate is the
session's `vision` flag: without it the image answers a header and an error naming the refusal, and a
file past the 4 MiB cap answers the same way. A file the head cannot name stays a text read and fails
as before when it is not UTF-8.

A file no sorter claims is streamed: only the window is held, so its whole text never lands in
memory, and for the same reason it grants no write licence — `splice` cannot write it.

A base64 payload that opens with an image's magic and runs past 512 characters prints as
`base64: N KiB masked`, its decoded size, while the rest of the line stays as it read, so a notebook
keeps its `"image/png"`. The mask is a printing fact alone: `splice` still addresses the line by
number, and the stamp keeps the bytes on disk.

A payload written one line per 76 characters — how Jupyter stores an image — folds instead: the run
becomes one line reading `base64: N KiB masked (M lines)`, and the lines under it are spent rather
than drawn. Every line keeps its own number, so a fold shows as a gap and the count of what it hid
is on the page; `splice` anchors, the stamp and the byte budget are all as they were.

A file a sorter claims is read whole only up to 4 MiB; past that it is streamed the same way, with
every line numbered, and it too grants no write licence. Any file past 16 MiB is refused outright,
since one line of it could still fill memory.

### A directory's entries

A directory answers one level: `name  N lines` for a file, `name/  K files, F dirs` for a directory.
Inside the source tree each child directory is opened one level too, folding to counts at ten files.
The session root lists its own entries, then every source tree the workspace's languages claim (`src/` for Rust, `adapters/` for Python), each as a section of its own with that tree's children beneath it. Build junk — `target`, `node_modules`, dot entries — is named like anything else and never opened.

A listing is never cut. Each level folds itself: a directory of ten files or more answers its counts
and nothing inside, so the answer is bounded by the depth times ten lines however many entries the
tree holds. The header carries the listed directory's own counts.

#### When a file carries its stamp

A level is one directory's own files, and every level is judged alone. Only files its language does
not call source take a part (`.md` for Markdown):

| kind | what it is |
|---|---|
| a document | upper case: no lower-case letter in the stem, at least one upper-case one |
| a kept document | a document whose stem the user listed in `config.server.agent.tools.read.persistentDocs` |
| anything else | every other file, and every directory |

A level whose documents are all newer than every other file beside it is vouched for: no file at
that level carries a stamp, and each document says what it is instead.

```text
  BUG?.md(latest)  1 lines
  code.rs  3 lines
  DESIGN.md(persistent)  2 lines
  README.md(latest)  2 lines
```

Any other level stamps every file at that level, and only a kept document keeps a tag.

```text
  2023-11-15 06:13:21  BUG?.md  1 lines
  2023-11-15 06:13:40  code.rs  3 lines
  2023-11-15 06:13:21  DESIGN.md(persistent)  2 lines
```

A kept document is never weighed and never vouched for: it cannot earn `latest`, it cannot keep its
level from being vouched for, and it is stamped like any other file when the level is not. A level
holding no document at all is vouched for, so an ordinary source directory answers as it always
did. A file whose moment cannot be read leaves its level unjudged, which stamps the level.

### Item mode

An item that names `item` turns its path into a scope: every piece under it whose own header equals
that text answers whole, every line numbered, in line order. What is matched is the header as the
file writes it, up to its `{` (`impl Shout for Dumb`) — and the match is equality, not containment.
Runs of whitespace collapse on both sides; nothing else is normalized. Copying the header from the
first line of an earlier answer is the way to name one: it prints that header itself.

A piece's own line and its span follow in parentheses: `fn walk(&self) (impl List)  2..5`. A
directory scopes every file a language reads under it, and each answer's file is named above its
first piece: `==> src/a/b.rs`.

`offset` and `limit` belong to the body read: `item` replaces the window, so both are ignored, and
the header says so in one line when the call stated either. The answer is not budgeted — equality
bounds how many pieces can share one header — so nothing is ever folded away here. Every file it
drew from is stamped as read, as a body read stamps one, so `splice` needs no second read.

### The module view

A file read inside a unit arrives with its neighbourhood, because a unit is edited as a whole far
more often than one file is. The graph layer builds it from the language's own facts; a language
without facts has no view. The view follows the body, opens with `--- module a::b (src/a/b/) · primer ---`,
and holds five parts:

- `exports (N):` — the entry's `pub` items and `pub use` statements, all of them; `none` only when
  it has neither.
- `down (N):` — the units the entry declares, one line each: label, size (files for a directory,
  lines for a leaf), visibility, `hub` or `leaf` (whether it declares units of its own), its
  exports, and the sibling units it reaches into.
- `out (N):` — what this unit itself reaches: the line and the symbols, an external target marked
  `(external)`. A `pub use` stays in `exports` and is not repeated here.
- `up (N):` — the line that declares this unit, then every file outside it that imports it, with
  its line, the symbols it takes, and `via` when a forwarding entry carried the name.
- `[scan: up→… | down→… | N levels, M files]` — where each search stopped and how much it saw.

The tag on the opening line says why the view is there: `primer` when it rode along on the first
read in a unit, `entry file` when the entry file itself was read. Each unit gets one primer per
conversation, and a compaction clears that memory. A file this same call already answered shows
`[body in this batch]` instead of its exports.

The view carries no budget of its own: a body that fills its cap still comes with a whole view.
The upward walk covers the next directory up and everything beside
it, one whole level at a time, and stops at the crate root or when its 300 ms budget runs out. A
level it could not finish is dropped whole, because half a level reads as "nobody imports this".
Past 200 importers the list folds by directory. The downward side never walks: it reads the names
the entry declares and nothing inside them.

## 2. Design intent

**Effective output over complete output.** Every read spends the model's context, so an answer
carries what a change needs and nothing else: the block edges a `splice` call can name, the
neighbours a module is edited against, the newest documents a level already vouches for.

**One shape for every answer.** A header names the path, a body shows what was read, a footer states
what was left out: the model reads the same three places whatever it asked for, and never has to tell
a truncated answer from a whole one by guessing.

**The entry file is the hinge.** `mod.rs`, the dual-track `x.rs`, `lib.rs` are what a crate is read
through, so the graph is found from the source tree outward and never by asking a compiler.

**A directory answer is two levels deep.** One level says what is here; the second says where a
module's files are, which is the question the model asks next.

**A stamp only where a judgment is needed.** A level whose documents are newer than its code needs
no date to be trusted, and the tool says so instead of printing twenty timestamps the model must
compare itself.

## 3. Unresolved

1. `pub use x::*`, `include!`, `#[path]` and `cfg` are not followed on the way up.
2. A module whose directory has no entry file is read by file names alone, and the view says so.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.read.persistentDocs` | the document stems the user keeps current: upper-cased, `.md` understood, default `DESIGN` |
