# edit

One unique run of text replaced in place: `old` must match exactly one place in the file, and
`new` takes its place. A call that cannot point at one run is refused, so an ambiguous match is
never written somewhere the model did not mean.

## 1. Design / Schema

### One call

`path`, `old`, `new`. `old` is one unique run of the file's text; a run that appears zero or
several times is refused. `new` is whatever stands in its place. `unfold` lifts the shared check's
fold for every written file of the batch.

`edit` takes no array: its payload is large enough that a batch would move the same bytes into one
argument. One call is one path in, one read, one atomic write. The file must already have been read
in this conversation: a path this conversation has not read is refused, and so is one whose last
write did not land clean. A refusal takes the licence away, so the next attempt must read or grep
the file first.

A session gets either `edit` or `splice`, never both; the rule is in `../../../README.md`.

### The answer

One line names what went, what took its place, and how far the rest moved:

```text
replaced lines 12-14 (3) with 5 lines; below +2; file now 144 lines: /p/src/foo.rs
```

### The batch's audit

A written Rust file joins the batch's audit like `write` and `splice`: each error and warning stands
under the answer, on the lines the change now occupies, and the crate's list carries only what no
written call could hold. A stop leaves the file untouched.

### The memo

```text
excerpt: { "atLine": 12, "excerpt": { "from": 8, "lines": [...], "gap": [8, 9, ...] } }
```

The memo describes one change: `atLine` is the old-file line it starts on, `excerpt` the item as it
stood before the change — its lines, and the old-file line each of them stands on — or a window of
five lines above and below the change when no item holds it. The excerpt never reaches past the item
the change sits in, and a folded payload inside it is one line standing for its whole run.

The item is the nearest enclosing block that declares something; failing that, the nearest one
that starts after a blank line, a comment or an attribute; failing that, the nearest block. Blank
lines never end an item, and an item's own closing brace belongs to it. The front end renders the
excerpt, signs the changed lines, and paints the rest.

## 2. Design intent

- **One unique run.** The model quotes what it changes instead of counting lines; a match that is
  not unique is a question, not a write.
- **One path, one read, one write.** The tool holds the per-path lock across the whole change, so
  no other call sees a half-applied edit.
- **A read is the licence.** The tool refuses a path the conversation has not read, and checks the
  same licence `splice` checks; the model cannot write by guessing text.
- **The receipt is the proof.** The answer says what moved below the change, so the model does not
  re-read the file to learn the new coordinates.

## 3. Unresolved

None.