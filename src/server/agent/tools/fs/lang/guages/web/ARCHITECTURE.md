# web — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

1. `mod.rs` — the language's registration: the four suffixes, the sorter dispatch, and the shape a line reads by.
2. `html.rs` — the page: tag scanning, the elements that own a piece, the id-carrying tags, the stylesheets and scripts the page pulls in, and the line ranges of its `<script>` and `<style>` blocks.
3. `css.rs` — the stylesheet: the scan of comments and strings, the rule and at-rule blocks, its imports and its selectors.
4. `js.rs` — the script: the scan of comments, literals, templates and regular expressions, the lines that carry code of their own, its declarations, class methods and control blocks, its imports and its exports.
5. `facts.rs` — the graph facts: the page root, one file per unit, and the edges a page, a stylesheet and a script write.
6. `view.rs` — the module view's narration, with a missing target marked where it stands.
7. `relations.rs` — the relation index: one DOM anchor read from all three kinds, a class member, a lone name.
8. `format.rs` — the parser: one `biome` round trip on a text handed over on stdin, under the deadline, the switch that starts the formatter of that kind, the one position it names when the text does not parse, and where the binary is looked up.
9. `check.rs` — the checker: the page root, biome's lint report, and the word for a pass that read none of the written files.
10. `layout.rs` — the commit's own round trip, on the same binary and the same deadline, with no switch of its own.

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its parser, its checker and its layout.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts, parser, layout and checker. It names no tests.
- The graph facts feed the module graph; the narration is the language's own.
- The three sorters answer `mod.rs` alone: a page, a stylesheet and a script each keep their scan to themselves, and the facts and the relations read them through that one door.
- The parser and the commit share one round trip in `format.rs`, and the deadline with it; the switch that starts a formatter is the parser's argument alone.

## Must stay in step

1. The four suffixes are one language. Splitting a kind into a language of its own ends the three-way anchor index, whatever else it may buy.
2. A page's root is the nearest ancestor serving `index.html` or `index.htm`; the graph's up walk and the checker's unit both read it from `facts.rs`, never from a second rule.
3. Every tag the sorter pairs must close in the table it builds; a page that never closes an element owns the rest of the file, and no other piece may claim those lines.
4. The four suffixes are the four names biome reads a kind from, so a file is handed over under its own name; the parser names the kind's formatter switch, and the checker and the layout keep the repository's own answer.