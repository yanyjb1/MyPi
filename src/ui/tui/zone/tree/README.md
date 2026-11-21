# tree — the block tree

A full-screen navigator over the blocks of one conversation: one block, one row. It draws inside
the shared page chassis (`zone::page`), the same frame the session picker uses.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `structure: Structure` | the last structural window handed down |
| `blocks: BTreeMap<i64, Node>` | what each block says, as the answers carried it |
| `rows: Vec<Row>` | the projection, rebuilt only when the inputs moved |
| `visible: Vec<usize>` | indices into `rows` that passed the filter |
| `filter` | the search words, matched against role and summary |
| `selected` / `scroll` | position inside `visible` |
| `only_user` | `Tab` keeps user messages only |
| `cherish` | `config.ui.tui.zone.tree.cherishYourWords`: Enter and Shift+Enter swap |
| `lang` | the display language; `en` until the config answers |

`Node` is the projection of one block: its kind in the tree's words, a one-line summary, and the
body when it is a user message. `Row` adds the geometry the layout computed.

### Header

The page's head line is `blocks · <rows> · <rounds> rounds · <scope>`, where the scope is
`every block` or `user messages`. Under it come the filter box and its caption; the key hints wait
at the bottom, under the rows.

### Filter box

The filter has its own box: a rule above it, the prompt and the reader's words inside, a rule
below it, and a caption under that saying what the filter looks at. The caption names the role and
the summary, because those are the only text a row carries. The box, the body and the hints are
inset from the frame's sides by a margin the chassis computes, so the content never sits against
the border.

### Rows

`source::rows` walks the structure's ranges, joins each block id with its node (a block the
window did not carry still gets a row, without a summary), filters `only_user`, then lays the
result out with `layout::rows`. The geometry is a single fork today. The run covers the whole
conversation, and the draw scrolls it: the cursor never leaves the screen.

The filter narrows `visible` and never reorders `rows`. It matches what the tree already holds:
a block whose contents never arrived has only a placeholder for text, so it cannot match a
summary. That is deliberate — the page asks for a screen at a time, and filtering nothing that
is not already on hand keeps it that way.

The first build puts the cursor on the newest block; after that it stays where the reader put it.

### Blocks on demand

The structure carries every block id; the contents arrive later. Whatever the visible window
shows and does not hold yet is asked for as closed id ranges, one `call blocks` per screen.
Ranges already asked for are remembered until the structure changes, so a scroll never repeats a
question.

With `only_user` on, a round's first block is kept as a row even before its contents arrive; every
other row is kept only once it is known to be a user block. So the filter costs no extra fetch.

### Keys

| key | does |
|---|---|
| `↑` / `↓`, `PageUp` / `PageDown`, `Home` / `End` | move |
| wheel / `Shift+wheel` | one block / ten |
| `Tab` | toggle user messages only |
| `Enter` | rewind to the row; `Shift+Enter` focuses it in the transcript instead |
| `Shift+Enter` | with `cherishYourWords` on, rewinds instead of focusing |
| printable characters, `Backspace`, `Ctrl+U`, paste | edit the filter |
| `Esc` | back to `Main` |

Focusing writes `tree.focus` = the block id into the handover memo; `Main` hands it to the
transcript, which owns the round lookup. The bottom hint lists these keys; the words for the two
that swap follow `cherishYourWords`.

### Requests

`start` asks for the structural window, `cherishYourWords` and `config.ui.lang`; `Enter` (rewind)
sends `post meta` with `rewind`.

## 2. Design intent

- **Structure plus content.** The ranges say which blocks exist and which round owns them; the
  answers say what they say. The tree stores both, joined on the block id.

- **Rebuild lazily.** A flag says the rows are stale; the draw rebuilds them once.
- **Leaving a note, not a call.** The tree tells `Main` where the cursor was through the memo.
- **One frame for every page.** The edge, the head, the filter box and the key hints come from
  `zone::page`; a page supplies its title, its filter placeholder, its caption, its empty word and
  its hints.

## 3. Unresolved

1. Multi-fork projection: the geometry supports it, the data source does not exist.
2. The filter cannot see blocks the page never fetched; see above.

## 4. Config fields

| key | use |
|---|---|
| `config.ui.tui.zone.tree.cherishYourWords` | `true` moves rewind to `Shift+Enter` |
| `config.ui.lang` | the language the head, the hints and the role names are written in |