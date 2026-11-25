# web — design

The web as one language: pages, stylesheets and scripts, read and edited together.

## 1. Design / Schema

- Suffixes: `.html`, `.htm`, `.css`, `.js`. All four belong to this one language, so a single query can cross a page, a stylesheet and a script.
- Projects: none. No marker file claims a directory, so a sweep of the workspace never stops at one; a file's nearest ancestor holding `index.html` or `index.htm` is the root its paths are read against, and the file's own directory when there is none.
- A unit is one file: a page is named by its directory when it is that directory's page, not by its file name.
- The line table reads, per kind: a page's structural elements, the tags carrying an `id`, and the `<style>` and `<script>` blocks they carry; a stylesheet's rule blocks and at-rule blocks, nested ones included; a script's `function`, `class`, top-level `var` and `import`, the methods of a class, and the control blocks inside a long body.
- Graph facts: a page is the hub. Its `<link rel="stylesheet" href>` and `<script src>` are its out edges, a stylesheet's `@import` and a script's `import … from` likewise. A path opening with `/` is read from the page root, anything else from the referring file's own directory; a target that is not on disk lands nowhere and the edge keeps the path it wrote. A named file that exists is an asset of the unit naming it.
- Exports: a page's ids, a stylesheet's selectors, a script's `export`. Past twelve names the rest fold into a count.
- Relations: `#id` and `.class` answered from all three kinds at once — the page's tags, the stylesheet's rules, the script's `getElementById`, `querySelector(All)` and class-list calls; `Class.method` and a lone name answered inside scripts.
- Parser: `biome format`, driven on stdin, and read for its verdict alone: a page is always handed over with `--html-formatter-enabled=true`, since biome leaves the HTML formatter off until asked. What it lays out is thrown away, so a written file keeps the lines the model sent.
- Checker: `biome lint --reporter=json`, over the files a batch wrote; a `.htm` page, a name biome does not know, is handed over on stdin as `.html`. Every finding is a warning, never an error.
- Tests: none.

## 2. Design intent

- **Four suffixes, one language.** A DOM anchor is the seam where editing goes wrong: a renamed id breaks a page, a stylesheet and a script in one move. Three languages could not answer that with one query, because a relation index only sees the files of its own language.
- **The page is the hub.** The graph follows what a page pulls in, and the relation index reads all three kinds together, so a stylesheet or a script is never read alone.
- **A missing target is said, not raised.** A `<link>` pointing at a file that is not there is a fact about the page, shown in the module view; the checker stays what it is — a report on the files a batch just wrote.
- **The parser is not tuned.** It answers one question — does this parse — and biome's defaults are the rule it answers by. Neither indentation nor line width is negotiated per repository.
- **A broken tag chain stays readable.** A page that never closes an element still yields the elements around it: an unclosed tag owns the rest of the file rather than swallowing the whole table.

## 3. Unresolved

- The upward walk reads whole directories, never the files standing beside the one being viewed, so a file that mounts another from the same directory is not seen as its importer. That is the shared walk's own reach, not a rule of this language, and it holds for every language alike.
- A `<style>` or `<script>` block is read as one piece of its page, never as a file of its own: a query inside it sees the page's kind, not the block's.
- Selectors are read as written, so a class named only by a compound selector or an attribute is not an anchor.
- A path built at run time, or a template-literal `import()`, is not followed: the graph is a signpost, not a bundler.
- biome must be on PATH; a repository that keeps it in a local package tree is not found.

## 4. Config fields

None. The web reads no key of its own; the toolchain is found beside this executable or on PATH.