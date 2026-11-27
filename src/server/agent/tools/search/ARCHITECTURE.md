# search — architecture

## Files

1. `mod.rs` — the tool entry: the same `WebArgs`, and one query against one engine.

## Relations

- Depends on `crate::browser` for `WebArgs` and `run`, and the engines and their parsing stay there.
- Consumed by the engine and the registry like every tool.
- Kept in step with `src/browser/README.md`, since the provider names and the answer shape are the domain's.

## Must stay in step

1. The `provider` values and the domain's provider set, so a new engine starts in both.
2. `pages` and the domain's page walk, because the tool passes the count and the domain decides when a page adds nothing new.