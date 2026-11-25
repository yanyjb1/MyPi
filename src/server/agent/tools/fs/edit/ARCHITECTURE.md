# edit — architecture

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the tool export |
| `tool.rs` | the tool's face: its parameters, the licensed read, the unique-run match, the one-line receipt, the excerpt memo and the audit handover |

## Relations

- Depends on `workspace` for the licensed zones, the per-path write lock and atomic writes; the read, the match and the write happen inside one lock, and the read must already hold the conversation's licence.
- Depends on `fs/lang` for the line table's pieces, so the memo's item comes from the same structure the other file tools read.
- Depends on `fs/change` for the excerpt around the change and the receipt's line arithmetic.
- Hands its written shell to the batch's audit like every written call; the diagnostics land under the answer that wrote the lines.
- Consumed by the engine and the registry like every tool.
- Kept in step with `fs/splice`: either name may be in a session's list, never both; the mutual exclusion is settled in the profile's tool list and recorded in the tools README.

## Must stay in step

1. The item a memo draws and the piece the language reports are the same piece; a change to what counts as a piece reaches both.
2. The receipt's line arithmetic and the write it describes come from the same change; a change to the landing rule changes both together.
3. A memo never reaches past the item the change sits in: the window is the fallback only when no item holds the change.