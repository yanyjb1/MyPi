# splice — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations, the tool export, and the two names `final` reuses |
| `tool.rs` | the tool's face: the parameter table, the batch-aware run (authorisation, one merged write per file, one answer per call, the file's one item outcome, the audit's later half), the merged snapshot and the flattening of every call's items |
| `edit.rs` | one range: its snapshot coordinates, its block, the macros it fills, the ranges it cuts, the anchor and overlap rules, the reverse-order splice and the whole-line write `final` reuses |
| `fmt.rs` | the language's formatter through the gate: the merged text handed over as one draft, and the landings the language answers |
| `receipt.rs` | the answer: the verdict line, one block per range, the word a refused range carries, the segment memo and the footer's shared report |

## What it talks to

- **The tool environment**: the workspace and its per-path write lock, the read stamps, the batch, the turn's stop and the shells.
- **The call view**: the one-line history summary.
- **`fs::lang`**: the line table, the formatter and the checker.
- **`fs::fault`**: the handover by which the compiler's faults reach the answer of the call whose lines they hit.
- **`fs::workspace`**: atomic writes and the per-path write lock.
- **`fs::change`**: the block's lines and the receipt `final` reuses.
- **The platform**: the turn's stop signal and the process group a formatter is killed with.
- **The tool contract**: parameters, argument kinds, the schema and the flat comparison an anchor uses.
- **Who calls it**: the registry holds the tool; `read` asks whether a body carries line numbers at all; a child's draft reuses the whole-line write.

## Must stay in step

1. The two anchor words and the drift rule: a bare number names a blank line only there, and the tool's own schema states it.
2. The edit item's kinds and both fields: the schema renders an integer or a string, so the parser and the referee are two halves of one rule.
3. The formatter's landings and the answer's rows: the answer names each block by the coordinates the language's formatter answered, so a changed answer reaches both. How a text is walked back to its blocks belongs to the language alone.
4. The checked file, its landing places and the per-call slots are built together, and the audit reads them as they are.
5. The receipt's footer tags and the audit's report lines are one family, since both land in the same footer.
6. The authorisation is granted only for a clean `OK`, so the batch's clean state and the answer's verdict must agree.
7. The whole-line write and the child's final draft: they are the only other consumer of that write, and they carry no anchor words at all.
8. The tool's name and `read`'s line-number gate: a body carries line numbers only while `splice` is in the session's list.