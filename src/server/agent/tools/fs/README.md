# fs

The file tools: `read`, `grep`, `edit`, `splice` and `write`, with the audit, process gate, change
context and report skeleton they share.

## 1. Design / Schema

- A session gets `edit` or `splice`, never both; naming both keeps `splice`.
- Every written file joins the batch's audit: one check per unit after the last write, and one test
  run for any test the batch declared.
- Every external program runs through the process gate: one child leads its own group, and a
  deadline or the turn's stop kills the whole tree.
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

## 3. Unresolved

None at this layer.

## 4. Config fields

None at this layer; each tool lists its own.