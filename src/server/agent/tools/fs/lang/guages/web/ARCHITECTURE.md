# web — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | the language's registration: the four suffixes, the sorter dispatch, and the shape a line reads by |
| `html.rs` | the page: tag scanning, the elements that own a piece, the id-carrying tags, the stylesheets and scripts the page pulls in, and the line ranges of its `<script>` and `<style>` blocks |
| `css.rs` | the stylesheet: the scan of comments and strings, the rule and at-rule blocks, its imports and its selectors |
| `js.rs` | the script: the scan of comments, literals, templates and regular expressions, the lines that carry code of their own, its declarations, class methods and control blocks, its imports and its exports |
| `facts.rs` | the graph facts: the page root, one file per unit, and the edges a page, a stylesheet and a script write |
| `view.rs` | the module view's narration, with a missing target marked where it stands |
| `relations.rs` | the relation index: one DOM anchor read from all three kinds, a class member, a lone name |
| `format.rs` | the formatter: the marker style each run needs by where it stands, the stdin call to biome, and where each written run landed |
| `check.rs` | the checker: the page root, biome's lint report, the `.htm` pass that reads a page on stdin, and the one place the binary is looked up |

## Relations

- Registered in `lang`'s language list; no tool names it.
- Uses the shared process gate for its formatter and its checker.
- Answers the registry's contract: the line table, the language's own shape, relations, graph facts, formatter and checker. It names no tests.
- The graph facts feed the module graph; the narration is the language's own.
- The three sorters answer `mod.rs` alone: a page, a stylesheet and a script each keep their scan to themselves, and the facts and the relations read them through that one door.

## Must stay in step

1. The four suffixes are one language. Splitting a kind into a language of its own ends the three-way anchor index, whatever else it may buy.
2. A page's root is the nearest ancestor serving `index.html` or `index.htm`; the graph's up walk and the checker's unit both read it from `facts.rs`, never from a second rule.
3. A written run's marker style follows where it stands, not only the file's suffix: markup takes `<!-- -->`, and a run inside a page's `<script>` or `<style>` takes that block's own comment, because a tag comment inside a script is a syntax error. biome must keep those markers for the run to be found again.
4. Every tag the sorter pairs must close in the table it builds; a page that never closes an element owns the rest of the file, and no other piece may claim those lines.
5. `.htm` is not a name biome knows: the formatter and the checker both hand such a page over as `.html`, and the checker maps the report back to the file that was written.