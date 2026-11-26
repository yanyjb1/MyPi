# yaml

A `.yml` or `.yaml` document: every key and every item is a piece, and the only question the parser asks is whether the text is legal YAML.

## 1. Design / Schema

- Suffixes: `.yml`, `.yaml`.
- A key line is a piece labelled `key`, and a `- ` line is a piece labelled `item`. A key whose value stands on its own line reaches over that nested body; every other key owns its own line.
- An item's own key is the item's name, so `- uses: actions/checkout@v4` answers to `uses`.
- A block scalar (`|`, `>`, with any chomping marker) owns its whole body, and no line of that body is read as a key: the body is a string, and a `foo:` inside it is text. Its body reaches while lines stay deeper than its own marker line, so a sibling item or key at that depth ends it, and a bare marker — `- |` — is a piece of its own.
- Parser: `saphyr` in process, every document of the text. A text it reads as legal YAML is legal, whatever dialect the document was written for: GitHub Actions' `on:`, docker compose, a Kubernetes `!Custom` tag, an Ansible `!vault` block, a GitLab `!reference`, `%YAML` directives, `<<: *anchor` merge keys and several `---` documents all read.
- No layout, no checker, no tests, no graph, no relations, no project marker: a commit leaves the file exactly as the model wrote it.

## 2. Design intent

- **Legality, not dialect.** A workflow file, a compose file and a Kubernetes manifest are ordinary YAML; they need no dialect of their own. What the parser refuses is what no YAML parser would accept — a tab as indentation, a key written twice, a quote left open — and those are exactly the mistakes a hand-written config makes.
- **A template is not YAML.** Helm's `{{- if }}` standing on its own line is not legal YAML before it is rendered, so it is refused. That is the parser doing its job, not a gap: a file that is not YAML until a renderer touches it has no syntax a YAML reader can judge.
- **Indentation is the structure.** YAML's own hierarchy is its indentation, so a key's reach is decided by the lines indented deeper than it, and nothing else.
- **Duplicates are errors.** The parser refuses a key written twice in one mapping, which is the classic copy-paste accident in a long workflow file.

## 3. Unresolved

- A flow collection written across several lines (`key: [a,\n b]`) has its entries on lines no piece numbers.
- A tab is counted as eight columns by the sorter while the parser refuses it outright, so a text the parser rejects still folds into a shape.

## 4. Config fields

None. YAML reads no key of its own.