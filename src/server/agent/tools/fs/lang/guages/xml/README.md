# xml

A `.xml` document: each element's start and end tags are its piece, and the parser judges the pairing.

## 1. Design / Schema

- Suffix: `.xml`.
- An element is a piece, from the line its start tag opens on to the line its end tag stands on. A self-closing tag is one line. A comment, a CDATA section and a declaration are stepped over whole and number nothing.
- An element's exact is its tag name, and, when it carries one, its `id` or `name` (`item#a`), so two elements of one tag can be told apart when a model copies a header.
- Parser: `quick-xml` in process. It names a mismatched closing tag; an element left open at the document's end is caught by counting the tags the parser opened and closed.
- No layout, no checker, no tests, no graph, no relations, no project marker: a commit leaves the file exactly as the model wrote it.

## 2. Design intent

- **Pairing is the syntax a model breaks.** An XML document's own structure is its element tree, and a hand edit that drops a closing tag or closes the wrong one is what makes it unreadable. That is what the parser answers.
- **Attributes are not judged.** Whether an attribute's value is quoted, and whether a document carries two roots, are rules `quick-xml` does not enforce; a model editing an element's body rarely breaks them, and a stricter judge would cost a second parser for a fault it almost never sees.
- **Two elements of one tag need names.** Unlike a source language, XML repeats one tag at every level of a document, so an anchor that copies only `<item` would answer for every item in the file. The `id` or `name` an element carries is what tells them apart.

## 3. Unresolved

- An attribute's quoting and a document's single root are unchecked.
- A tag and its end written on the same line as their content is one piece on that line, so a nested element sharing that line cannot be named separately.
- Namespaces are read as part of the tag's text rather than split off, so `x:item` answers to `x:item`.

## 4. Config fields

None. XML reads no key of its own.