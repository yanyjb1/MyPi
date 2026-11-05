# command — architecture

What is here, who reads it, and what it never touches. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list and the re-exports | public |
| `meta.rs` | `CommandMeta`, `Arg`, `Candidate`, `lookup`, `split`, `valid` | public |

## What it talks to

- Nothing. The module's only import is `serde` on `Candidate`.
- **Knows nothing about**: front ends, the session, the api, doors, terminals, configuration.

## Who calls it

- `ui::tui`'s input zone: `command/table.rs` holds the table and calls `valid` in a `const`;
  `route.rs` classifies one line with `split`; the completion engine and the hang check use
  `lookup`; the pool decoder builds `Candidate`s.
- Nothing else, today.

## Must stay in sync

1. `Arg`'s variants and the completion engine's match over argument positions.
2. `valid`'s rules and every table: a violation fails the table's own compile.