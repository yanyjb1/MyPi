# cp

CP's own declarations, registered in `mod.rs`: one domain over both halves — C is the half without
types and namespaces, C++ the half with them — so a header never has to say which half wrote it.

## 1. Design / Schema

- Suffixes: `.c`, `.h`, `.cc`, `.hh`, `.cpp`, `.hpp`, `.cxx`, `.hxx`, `.inl`, `.ipp`.
- A project is found by `compile_commands.json`, `CMakeLists.txt`, `Makefile` or `meson.build`,
  whichever is nearest, then by `.git`, then by the file's own directory.
- A unit is one file, named by its path from the project root. A header belongs to no unit but its
  own: that is what makes one language necessary rather than two.
- The line table reads `namespace`, `class`, `struct`, `union`, `enum`, `using`, `typedef`, function
  declarations and definitions, `extern "C"` wrappers and comment runs. Preprocessor lines are
  blanked before any brace is counted, so a macro body never opens a body.
- One grammar cuts both halves: C is the part of C++ whose headers already parse under it.
- Exports: the declarations at a file's own level, and whatever a `namespace` holds.
- Edges: every `#include`, resolved beside the including file first for a quoted name, then through
  the include directories the project's compile database names. An angled name never looks beside
  the file that wrote it.
- Parser: `clang -fsyntax-only` over stdin, with the project's include directories and the standard
  a lone text is read under. It is read for its verdict alone, so the written file keeps the lines
  the model sent.
- Checker: `clang-check -p` over the files a batch wrote, one translation unit each.
- Tests: none. CP carries no test framework of its own.

### Relations output

The relation index prints two sections for a two-segment query, in this order:

- `[type] A::b` — a type's member, whether declared in its body or defined outside it, or why no
  `A` in this project declares `b`;
- `[scope] A::b` — a declaration a namespace `A` holds, or why no namespace `A` exists.

A lone name prints `[x-ray] A`: every declaration of `A`, the scopes it stands in, and every
`#define` of it. At most 12 members are listed per type, and at most 8 declarations per name.

## 2. Design intent

- **One domain, two halves.** Merging them is what removes the `.h` question: a header may be
  included by a `.c` and a `.cpp` at once, and a language that owned only one half would have to
  guess.
- **The database decides what can be known.** Include directories, and therefore the graph's reach,
  and the checker's flags, all come from the build the project already describes. A project that
  describes no build still gets its includes read beside the file that wrote them.
- **No judge, no judgement.** A missing `clang` reads as no verdict and lets the text through; a
  missing `clang-check` or a missing compile database reports that nothing was checked rather than
  refusing the write. A header this text was never given is the world's fault, never the text's.
- **A unit is a file.** CP declares nothing about what belongs with what; the compiler's own
  translation units are the only grouping the language has, and they live in the database, not in
  the source.

## 3. Unresolved

- A macro that expands to a declaration is not seen; only the text is read.
- An include found through `-isystem`, a generated header or a build directory that does not exist
  yet lands nowhere, and the edge keeps only what it wrote.
- `clang-format` is not used as a parser: it reflows text without building a syntax tree, so it
  accepts what does not compile. Whole-file layout is not part of this language.

## 4. Config fields

None. CP reads no key of its own; the toolchain is found beside this executable or on PATH, and the
project is described by its own compile database.