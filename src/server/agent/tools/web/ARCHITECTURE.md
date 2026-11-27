# web — architecture

## Files

1. `mod.rs` — the tool entry: arguments to `WebArgs`, and one call to the domain.

## Relations

- Depends on `crate::browser` for `WebArgs` and `run`, and every browser fact stays there.
- Consumed by the engine and the registry like every tool.
- Kept in step with `src/browser/README.md`, since the domain's config keys and operations are the ones this entry names, and a rename on either side is a rename on both.

## Must stay in step

1. The `type` values and the domain's `op` values, which are one enum under two names, so a new operation starts in both.
2. The fields this entry reads and `WebArgs`, because the tool's parameter table is the only place the model learns the names.
3. `Env::browser` and the `Setup` the domain takes, because the launch values pass through this entry untouched.