# fs

The file tools: `read`, `grep`, `edit`, `splice` and `write`, with the audit, process gate, change
context and report skeleton they share.

## 1. Design / Schema

- A session gets `edit` or `splice`, never both; naming both keeps `splice`.
- Every written file joins the batch's audit: one check per unit after the last write, and one test
  run for any test the batch declared.
- Every external program runs through the process gate: one child leads its own group, and a
  deadline or the turn's stop kills the whole tree.

- A session holding only the file tools — `read`, `grep`, `edit` or `splice`, and `write`, with
  nothing that could run a command — gains a commit. `write` queues one by writing
  `.commitmessage` at the workspace root; the batch commits once its check has passed, every file
  set in its own language's style, and the paths handed to git one by one. A file whose own
  uncommitted changes predate the session holds the whole commit back.
- A change carries its own context: the piece around it, its line numbers and a one-line receipt.
- Every written text keeps the line ending the file already had: a file whose lines end in CRLF is
  written back as CRLF, a new one as LF. The tools match and parse plain lines and put the ending
  back once, at the write.
- Reports are drawn from one skeleton: numbered source lines, the pieces of a line range and the
  seam around an error.

## 2. Design intent

- One language registry answers every tool, so a new language is one knowledge file and one registry
  entry.
- `edit` and `splice` require a read; `write` replaces a whole file and needs none.
- The audit answers once per batch, so every written call reads the same compiler result.
- A commit happens after the check, never before, so a batch that fails to compile never enters the
  history.
- The layout runs before the check, so the lines the check names are the lines the file now holds.
- A deletion needs the file read, and the text the read saw: a file the user changed since is theirs
  again.

## 3. Unresolved

None at this layer.

## 4. Config fields

None at this layer; each tool lists its own.