# write

`write` replaces a whole file and refuses nothing: it is the counterpart of `edit`, which refuses
an ambiguous match. The text that lands is always the text as sent; the parser only judges it.

## 1. Design / Schema

### One call

`path` and `content`. The file is overwritten whole, byte for byte as the caller sent it.

The file's parser is asked whether the content parses, and the answer names the line when it does
not; the content still lands, and the batch's check still runs — the report names the file again.
A parser that is not installed, or did not answer in time, says nothing about the text, so the
content lands with nothing to report.

`write` takes no array: its payload is large enough that a batch would move the same bytes into one
argument.

### Two names that are not files

In a session holding only the file tools — `read`, `grep`, `edit` or `splice`, and `write`, with
nothing that could run a command — two words mean something else:

- `content: __DELETE_FILE__` deletes `path`. The file joins the batch's commit if git tracks it, and
the answer says which. Nothing joins the batch's audit.
- `path: .commitmessage` at the workspace root queues a commit instead of writing a file: the
text is the message, and the batch commits once its check has passed. A second write in the same
batch replaces the first message, and nothing touches the disk.

Both need the session to hold no shell: with one, a model deletes and commits by itself, and these
names mean the literal file they spell.

### The batch's audit

A written file joins the batch's audit: each of its errors and warnings stands under the answer as
a source seam, once, and the crate's list carries only what no written call could hold. A stop
before the write leaves the file untouched and the answer says so; nothing is said about the
compiler when there is nothing wrong.

### The answer

```text
overwrote /p/src/foo.rs (2311 bytes)
```

## 2. Design intent

- **Whole-file replacement is unconditional.** A model that knows the whole file needs no anchor;
  the parser and the checker still judge what landed.
- **Nothing is withheld.** A parse failure is reported, but the content still lands, because the
  batch's check is the real verdict.
- **Nothing is laid out.** A tool that reformats the caller's text moves every line number the
  caller is about to use, so the parser is read for its verdict alone.

## 3. Unresolved

None.

## 4. Config fields

None. `write` reads no key of its own; the parser's and the checker's behaviour comes from the
language, and the batch's reports from `unfold`.