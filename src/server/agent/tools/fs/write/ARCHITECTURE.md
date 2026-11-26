# write — architecture

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the tool export |
| `tool.rs` | the tool's face: its parameters, the parser's verdict, the write and the compiler's report |

## Relations

- Depends on `fs/lang` for the parser and checker the file's language answers.
- Depends on `fs/gate` for the parser and checker processes, their deadlines and the group a stop kills.
- Depends on `workspace` for the licensed zones, the per-path write lock and atomic writes.
- Hands its written shell to the batch's audit like every written call; the diagnostics land under the answer as a source seam, once.
- Consumed by the engine and the registry like every tool.
- Depends on `git` for the two questions its ledger keeps: whether a file is tracked, and whether the user's own uncommitted work already sat in it.
- Reads the session's ledger to record what a file was before this call, and the pending commit its `.commitmessage` word sets; `fault` commits from there once the check has passed.
- Kept in step with `fs/splice`: both read a file's parser verdict and the language's own answer, and never rebuild them here.

## Must stay in step

1. The parser verdict and the language's answer are one answer; a change to what a language reports reaches both `write` and `splice`.
2. The audit's unit and the written file: the file is checked inside the crate that holds it, and one batch gives one check per crate.
3. The write lock and the atomic rename belong to the workspace; `write` never opens the target directly.