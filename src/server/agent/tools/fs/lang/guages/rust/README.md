# rust

Rust's own declarations, registered in `mod.rs`: its suffixes, line table, graph facts, formatter,
checker, tests and relation index.

## 1. Design / Schema

- Suffixes: `.rs`.
- The line table reads Rust's pieces: declarations, `use` runs, attributes and comments, closing
  braces, and block heads inside long functions.
- Relations: `A::b` as a type or module member, and a lone name's declarations.
- Graph facts: the unit a file belongs to, the units an entry declares, exports, edges and the source domain.
- Formatter: `rustfmt`, with the edition from the nearest `Cargo.toml`. Each written block is wrapped in a pair of marker comments before the text goes out, and read back from the laid-out text afterwards.
- Checker: `cargo check` over the unit a file belongs to.
- Tests: the tests a written line range declares, run alone.

### Relations output

The relation index prints three sections, in this order:

- `[type] A::b` — a type member, or why no crate-defined `A` declares `b`;
- `[module] A::b` — a module member, or why no module `A` holds `b`;
- `[x-ray] A` — the declarations of `A` and the `impl` blocks that carry its members.

At most 12 members are listed per block, and at most 8 `impl` blocks per type's x-ray.

## 2. Design intent

- Rust knows what an `impl` block is, so the relation limits live here, not in the tool.
- A formatter, checker and test run are the language's own; the shared side only schedules them.
- The graph facts speak files; the machine above speaks units.

## 3. Unresolved

- A macro-generated item is not seen; only the text is read.
- A test whose full path the toolchain does not list cannot be run by name.

## 4. Config fields

None. Rust reads no key of its own; the toolchain is found from the file's own tree.