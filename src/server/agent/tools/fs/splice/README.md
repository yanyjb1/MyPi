# splice

Whole lines replaced by number: many ranges of one file in one call, one snapshot, one write.

A range names itself by what it lands on, so a line the model counted wrong is refused instead of written somewhere else.

## 1. Design / Schema

### One call

`path` and `edits`, and `unfold` when the batch's check and test reports are wanted whole.

Each element of `edits` is one range `{start, end, new}`; the elements are answered in the order they were given, and a batch of calls on one file becomes one write.

| field | value |
|---|---|
| `start` | the range's first line, 1-based, then the words that line opens with |
| `end` | the range's last line, inclusive, read the same way; its words are checked when the range covers more than one line, and it is refused when it sits above `start` |
| `new` | what stands in the range's place, any number of lines; empty deletes the range |

A range always replaces at least one line: `start` and `end` name lines, and there is no position between two of them.

### An anchor says what it lands on

`start` and `end` read as a line number and then the words that line opens with, `12 pub fn run`.

The words are compared as a flat prefix — whitespace runs folded to one space, the line walked from its front — so a model may stop short of the whole line and cannot match the middle of one.

The `end` words are checked only when the range covers more than one line: a single line is confirmed by its `start`.

A bare line number is read as a blank line and nothing else: on a dense line it is refused, and the refusal quotes what the line actually reads and the shape to name it by.

A `@@CUT(a..b)@@` inside `new` carries no words of its own, because the macro names its lines.

### Authorisation

A file may be spliced only after `read` or `grep` has shown it in this conversation.

That showing is the authorisation, and it carries the content's hash: a file that changed on disk since is unauthorized, and so is a file whose last splice did not end `OK`.

Any splice that is not a clean `OK` — a refused range, an unreadable argument, a block the parser left out — takes the authorisation away at once, one missed range out of many included.

The model's way back is one: `read` or `grep` the file again, then send the edit.

### What holds a range back

| refused | why |
|---|---|
| a line that is not what the anchor says | the model read a different file, or a stale one; that range is left out and the rest land |
| a range overlapping one already accepted, in the same call or an earlier one | the later range loses; the earlier one is written and the rest land |
| a range that cannot exist: past the last line, `end` above `start`, two cut macros of one call cutting the same line | that item is left out; the rest land |
| a file not read or grepped in this conversation | unauthorized |
| a file changed on disk since it was read | unauthorized |

Overlap is judged first and alone: a range that loses to an accepted one carries no other reason.

A range whose lines are its own is then asked whether it still finds what it named.

Every refusal takes one range or one item, never the call and never the batch: the rest of it lands, and the verdict counts what did.

### The batch's one write

Every `splice` call of one message on one file is merged: the ranges arrive in call order, are measured against the file as it stood before the message, and land in one atomic write.

A later call's range that overlaps an earlier call's landing loses, because what the earlier call wrote is no longer what the later call's words describe.

The per-path write lock of `workspace` is held across the whole of it: the file's bytes are read, compared with the licence, and written back inside one lock, so no other call sees a half-applied batch.

### Filling macros

`new` may name lines of the file instead of repeating them.

`@@COPY(a..b)@@` stands for lines `a` to `b` of the snapshot, 1-based and inclusive; `@@CUT(a..b)@@` stands for the same lines and adds the change that removes them from where they were.

Marks that carry no range of this file are text, kept as written: a sentence that spells a macro out stays readable, and a macro naming a line the file does not have is not refused on its behalf.

Filling happens once, before any range is applied, so a token inside the copied lines is copied rather than followed.

A macro may sit anywhere in `new`; a range a macro names is cut once per call, and a change may not cut the range it replaces.

### The parser and the checker

Both are the file's language's own, and they answer different questions.

The parser is a judge, never a layout: it says whether the merged text parses, and what it said when it does not. The text that lands is always the batch's own splice, so the line numbers the model read are the line numbers it writes.

| outcome | what happens |
|---|---|
| the text parses | every block lands as it was sent |
| the text does not parse | each block is tried on its own, and only the blocks the parser blames are left out; the rest of the batch still lands |
| the parser is not installed, or did not answer in time | no verdict is reached against the text, so nothing is left out |
| the turn is stopped | nothing is written at all |

A block left out is reported under its own item with the parser's words, and counts as a refusal.

The checker answers well after the write: the batch's audit type-checks the unit once, and its findings are the warnings a model acts on. A text the parser refused still reaches it.

### The batch's audit

A written `.rs` file joins the batch's audit, which checks the crate once per batch and hangs every diagnostic under the block whose lines it hit.

A call that drew no blocks hangs them at the end of its answer instead, so a whole landing still carries the crate's verdict.

A test failure is listed above a check warning on the same block.

What no written call can hold goes to the crate's list on the batch's last written answer that can still take lines; `unfold: true` lifts that list's fold.

### The answer

Three parts, and nothing outside them: the header names the file and says what became of the call, the body is one block per edit, and the footer is the batch's reports and the next move.

A call that landed whole and clean draws the header and nothing else.

Its diff is what the model just sent, and the file it can read is the truth; the body exists for calls that partly failed.

The header is one line: `splice <path>: <verdict>`.

The verdict is `OK`, `PARTLY FAILED` or `FAILED, NOTHING WRITTEN`.

A call that never reached its edits names the reason instead of a verdict.

The file is the unit of outcome: the call answers one item, and one refused range fails it, so the card turns red for the whole file. A call that landed nothing keeps that item and carries `fail` besides.

A written block opens with where it landed, names the item, and numbers its lines by the file's own numbers — the plus side as it now stands, the minus side as it stood before the write.

A deletion has no plus side.

A block that lost for overlap says one word, and nothing else.

A block refused by its anchor opens with the range it now stands at, then the anchor's words.

A block the parser left out opens with the range it asked for, says it does not parse, then the parser's words.

A range a written edit of the same batch moved says so first; otherwise the line is that range alone.

An item that could not be read at all has no range to name and carries its reason alone.

Both sides fold in the middle past twelve lines, counted as `[+N lines]`.

No line is cut.

The footer follows the body one blank line down and holds only what the file itself cannot say.

The check's counts cover every diagnostic the crate's check found, whether it hangs under an edit or is listed here.

`unfold` lists them all instead of the first twelve.

### The memo

One segment per range, in the order the call sent it: the lines it took and the lines that stand there now, numbered on both sides.

A segment also carries the item around the change, taken from the file as it stood before the write: the change inside an item hands that item's own lines above and below it, with the change's own lines cut out. A change that covers an item whole, or that no item holds, carries no context at all — the memo never reaches past the item the change sits in.

A range that was left out carries positions and a reason, never lines.

A range left out carries no lines at all: its `fromLine` is the line it named, its `atLine` the line its own range starts on now, and `error` what stopped it.

A `@@CUT(a..b)@@` adds a segment of its own — the lines it removed, with an empty `new`, under the same edit number.

Segments that are not neighbours are drawn apart, and the card folds what it does not need. A front end lays them by the line each one names, never in the order they arrived: a call whose ranges were sent out of order still reads top to bottom, and a segment that named no line keeps its place last.

No line of the file as it stood is carried twice in one memo: a change reaches down only as far as the next change's first line, so an item between two changes rides the one above it. A side cut away entirely is sent as `null`, and the front end draws the gap itself.

The new line numbers are the batch's own arithmetic, since nothing laid the text out in between; the tool carries them rather than leaving the front end to guess.

### Stopping

A stop lands in one of two places, and the answer says which.

Before the write, the parser is killed and nothing reaches the file, so the call answers `FAILED, NOTHING WRITTEN` and `interrupted`; the file keeps its authorisation, since nothing about it changed.

After the write, the change stands and waits for the batch's audit: the answer is finished once the batch's last write has landed and the crate has been checked, and a check that never ran says `[i] check · interrupted`.

## 2. Design intent

**A line number is useless without an anchor.** A number says where a change starts only if the model's picture of the file is still true; the words a line opens with are the proof it is, and they cost a few bytes instead of a whole `old` block.

That is why this tool exists beside `edit`: it spends `read`'s line numbers instead of fighting them.

**One anchor rule for both ends.** `start` and `end` read the same way, so a range is never a number on one side and a quotation on the other; both name a line, and both are always there.

**One rule for overlap.** Every range is judged against the ranges already accepted, whichever call sent them.

The later one loses, the earlier one lands, and nothing else about the call changes; a special case per call would make the same mistake refuse different amounts of work.

**A refusal is placed, not guessed.** A refused range still owns the lines it named, and it shares none with a written range, so where it starts after the write is arithmetic over the written ranges above it — never a number carried over from the snapshot.

**A refusal is local.** One stale range is dropped and the rest of the batch lands; the model resends only what lost.

Authorisation goes with it, because the model's picture of the file is what failed.

**The header says the verdict, the blocks say the place.** A successful call carries no numbers past its blocks; a refused one carries exactly one sentence about where its lines went, and the same sentence is the instruction to read the file again.

**The parser judges, it does not rewrite.** A text that does not parse is written as it stands and the block the parser blames is left out, because a tool that reformats the model's lines moves every number the model is about to use. Whatever the model writes is what the file holds.

**One write per file per message.** Two calls on one file would each need a fresh read between them; merging them makes the second call's coordinates mean what the model thought they meant.

## 3. Unresolved

1. A range cannot name a blank line by a number `read` prints: `read` prints no number on a blank line, so the model counts one itself.
2. `@@COPY` and `@@CUT` take plain line numbers, with no worded anchor of their own: a macro naming the wrong lines copies them without a refusal.
3. Nothing checks that a `new` block belongs where it is put beyond the parser: a block moved into another function parses and lands.

## 4. Config fields

None.

`splice` reads no key of its own; the parser's and the checker's behaviour comes from the language, and the batch's reports from `unfold`.