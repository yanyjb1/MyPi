# go

Go's own declarations, registered in `mod.rs`: its suffixes, line table, module graph, relations,
parser, checker and tests.

## 1. Design / Schema

- Suffixes: `.go`. Project marker: `go.mod`, which the toolchain itself requires before it builds
  anything, so it is both the project's name and the condition of every check.
- The line table reads Go's pieces: `package`, `import`, `func`, `type`, `var` and `const`, the
  parenthesised groups of the last four, comment blocks, and the branches inside a long function.
- Relations: `A.b` as a package's declaration or a type's method and field, and a lone name's
  declarations. Go writes a method outside the type it belongs to, so a type's members are read from
  the whole file rather than from the type's own body.
- Graph facts: one package is one unit, named by its import path, and every import resolves to the
  package directory it names. A module holds packages, not modules, so the module is the domain.
- Parser: `gofmt -e` over the text itself. It is read for its verdict alone, so the text it lays out is
  thrown away and the written file keeps the lines the model sent.
- Checker: `go build` over the packages a batch wrote, and `go vet` on what compiled. A build failure
  stops the batch's check; a vet finding is a warning, because it compiles and still reads wrong.
- Tests: the `Test*` functions a written range declares that take a `*testing.T`, listed by the
  toolchain and run alone.

### Relations output

The relation index prints two sections, in this order:

- `[type] A.b` — a method or a field of the type `A`, or why no type `A` carries `b`;
- `[package] A.b` — the declaration package `A` spells, or why it declares none.

A lone name prints its own declarations as `[x-ray] A`.

## 2. Design intent

- A Go package is a directory, and every file in it compiles together, so a unit is a directory that
  names no entry file: its own files are its text.
- `go vet` says two different things on two different streams — a compile failure on its error stream,
  a reading complaint on its standard output as JSON — and this language keeps that difference instead
  of flattening it, because a batch must hear which of the two it is.
- A build of a `main` package would leave a binary beside the source, so the build target is thrown
  away: this tool never writes into the tree it reads.
- Go has no module statement to point at, so a child package is declared by where it sits and carries
  no line of its own.

## 3. Unresolved

- `go.work` is not read: one nearest `go.mod` is the unit, so a workspace of several modules reads as
  whichever module holds the file.
- A group of imports is one piece, so a change inside it is answered by the group's first line.
- A build tag or a file suffix that excludes a file from a package is not read: every `.go` file in a
  directory counts, and the toolchain alone decides what is really built.

## 4. Config fields

None. Go reads no key of its own; the toolchain is found beside this executable, then on `PATH`.