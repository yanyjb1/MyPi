# read — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | module declarations and the one re-export |
| `tool.rs` | the tool's face: the path array, the per-kind budgets and the per-item limit under them, one path's answer and the module view that follows a source body |
| `dir.rs` | directory listings: one level per directory, each level's stamp verdict, entry counts and the per-level fold |
| `items.rs` | the item mode: the pieces whose own header equals the one the model named, under one file or a whole directory |

## What it talks to

- **The tool environment**: the workspace, the read stamps, whether `splice` is allowed, vision and the persistent documents.
- **The call view**: the one-line history summary.
- **The workspace**: the canonical path form every comparison uses.
- **`lang`**: the line table, the graph facts a language supplies and the source-tree answer.
- **`lang::graph`**: one file's neighbourhood, printed by its language or by the machine's default.
- **The platform clock**: the local stamp a listing prints.
- **`splice`**: the gate that decides whether a body carries line numbers.
- **Who calls it**: the registry holds the tool; the front door runs it like any other tool.

## Must stay in step

1. Read's caps and the engine: the front door skips measuring for `read` because the tool cuts its own files; remove either half and a single-line file reaches the context whole.
2. The directory verdict and the persistent-documents config: a configured stem is compared exactly, so a name the config upper-cases still matches nothing on disk written lower case.