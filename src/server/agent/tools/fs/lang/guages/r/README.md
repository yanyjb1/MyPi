# r

R's own declarations, registered in `mod.rs`: its suffixes, line table, relation index, parser and tests.

## 1. Design / Schema

- Suffixes: `.R`, `.r`.
- A byte order mark, which an editor on Windows writes, belongs to no line of code and is dropped before the table reads a text.
- The unit is the nearest directory above a file that holds a `DESCRIPTION`, else the file's own folder. `DESCRIPTION` is read for its `Package:` line and nothing else.
- A definition written inside a call or a body belongs to whatever holds it, so the table names no control flow inside a function. An `if` that hands over to an `else` is one piece, the branches after it included.
- A piece's own text is the header with the body's opener dropped. A `function` signature is folded over the lines it continues onto; a call is named by its own first line.
- Relations: one name's declarations at the file's own level, bare or in backticks. `pkg::fun` is not read: a package outside this workspace resolves to nothing a search could find.
- Parser: `air format --stdin-file-path`, read for its verdict alone. The text is handed down its stdin and what `air` lays out is thrown away, so a written file keeps the lines the model sent.
- Tests: `testthat` through `Rscript -`, one test file at a time. A change to `tests/testthat/test-*.R` runs that file; a change to `R/<name>.R` runs `test-<name>.R` when the project holds one. A project with a `DESCRIPTION` is loaded first, and a project that will not load is reported as such rather than as failing tests.
- No checker: R has no type system, and a linter's findings are not what a batch needs to hear.
- No graph: an R script reaches the workspace through `source()` alone, which is too thin a signpost to be worth a module view.

### Relations output

A lone name prints its own declarations under one heading:

- `[x-ray] A` — where `A` is declared, each row a path, a line, the piece's own header and the lines it spans.

At most 12 rows are listed per source.

## 2. Design intent

- A definition is a binding, not a keyword: `f <- function(x)` puts the name on the left of an arrow and the kind on the right, so the sorter reads both sides instead of a head word.
- R's file system spelling is `.R`, and lower case reaches the same file on a case-folding disk, so both suffixes are claimed and one test path is asked for once.
- A test file and the source it tests are separate files by R's own convention, so the test a change asks for is derived from the source's name rather than found inside it.
- A package is loaded before its tests run: `testthat` alone cannot see the functions a project declares, and a test that cannot call them fails for the wrong reason.

## 3. Unresolved

- A definition bound through `assign()`, `do.call()` or a computed name is not read.
- `R6Class()` members are not pieces of their own: the call is one piece, so a change inside `public = list(...)` is answered by the call's first line.
- S4 generics and methods are named by their call, not by the class they belong to, so a query for a method's own name finds nothing.
- `pkg::fun` is not read, so a query naming a workspace package's function finds nothing either.
- `air` is not on PATH until it is installed; a project without it is never parsed and every text it holds is taken as it came.

## 4. Config fields

None. R reads no key of its own; the toolchain is found beside this executable, then on `PATH`.