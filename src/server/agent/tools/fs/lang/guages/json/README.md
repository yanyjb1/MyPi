# json

A `.json` document: its containers are pieces, and its own parser is the only judge of whether it reads.

## 1. Design / Schema

- Suffix: `.json`.
- A container is a piece: a `{`…`}` object or a `[`…`]` array, from the line its bracket opens on to the line its bracket closes on. The lines between carry no number of their own, so a document folds to its own shape.
- A container's exact is the key standing before its bracket (`"list"` for `"list": [ … ]`); an array whose element is itself a container numbers that element by the key of the array that holds it, and an object inside an array carries no key at all.
- Brackets are found by blanking every string body first, so a `[` inside a value opens nothing; the quotes stay, so a key still reads as a quoted key.
- Parser: `serde_json` in process. It names the line and the column of the byte it refused.
- No layout, no checker, no tests, no graph, no relations, no project marker: a commit leaves the file exactly as the model wrote it.

## 2. Design intent

- **One bundle, no dependencies.** JSON is the format a model meets most often, and it is the one language here whose parser was already a direct dependency, so it costs the build nothing.
- **The shape, not the content.** A document's own assembly is what a model needs when it opens a large file: which key holds what, and where that key starts. Numbering every entry would bury that under the content itself.
- **A comment is not JSON.** `.json` is read strictly; a comment is a refusal, and the answer names the line it stands on.

## 3. Unresolved

- A document written on one line is one piece holding the whole file, so no container inside it can be named.
- The key of a container is read from the text before its bracket, so two containers opened on one line number only the last of them by key.
- `.jsonc` and `.json5` are not claimed: both allow comments and trailing commas, which `serde_json` refuses.

## 4. Config fields

None. JSON reads no key of its own.