# hub — the subagent panel

A read-only page over the sub-sessions one conversation spawned: one block of three rows per
child, and under the list the blocks the chosen child wrote. It draws inside the shared page
chassis (`zone::page`), the same frame `resume` and `tree` use, and lists its rows through the
shared `Picker`.

It never becomes the child. A front end follows one conversation at a time, and leaving this one to
watch a child would unbook the conversation, its job pool and every background task it holds.

## 1. Design / Schema

### State

| field | meaning |
|---|---|
| `picker: Picker<Row>` | the `call children` answer, the cursor and the window |
| `peeked` | the child the preview belongs to |
| `structure: Structure` | that child's rounds, as `call init` carried them |
| `blocks: BTreeMap<i64, Block>` | the child's blocks in hand, by id |
| `asking: HashSet<[i64; 2]>` | the id ranges asked for and not answered yet |
| `peek` | whether the keys move the child list or the preview |
| `cursor` | the preview's cursor, as a block id |
| `loaded` | a list arrived at least once |
| `error` | the last list answer was refused |
| `lang` | the display language; `en` until the config answers |

`Row` carries the child's `id`, its `name`, its first user message, its round count, its transcript
size, its directory, and `running` — whether the loop still holds it.
`Block` carries one block's id, its role in the tree's short words, and a one-line summary.

### Layout

The head says how many children there are; the line under it says what the page is for.

The body is cut in two. The list may take half the room; whatever is left belongs to the preview.
A list shorter than its half gives the remainder away. On a frame too short for both, the preview
goes first and the list keeps what it needs.

One child is three rows: title, figures, then a blank. A running child's title is painted in the
live accent; a finished one is grey. The cursor is `> ` and its blank placeholder keeps every title
aligned.

### The preview

`call init` answers a whole conversation: its structural window, and its newest blocks. The window
says which block ids exist, so the preview reads the conversation in order and asks `call blocks`
only for the ids near the cursor it does not hold yet. Ranges already asked for are remembered
until the next `init`, so a scroll never repeats a question.

The draw ends at the preview's cursor and reaches back, so a reader follows the newest block.

### Keys

| key | does |
|---|---|
| `↑` / `↓` | move the list, or the preview's cursor |
| `PageUp` / `PageDown` | ten rows, or ten blocks |
| `Home` / `End` | first / last |
| `Enter` | read the chosen child: the keys move the preview from now on |
| `Esc` | leave the preview; from the list, back to `Main` |
| `Ctrl+X` | stop what the chosen child is running |
| wheel | the same one row |

### Requests

`start` asks for the list and `config.ui.lang`. Moving the list asks `call init` for the child now
under the cursor. A preview that needs blocks asks `call blocks`. `Ctrl+X` sends `post interrupt`
with the child's own id, then refetches the list.

A running child is read again every two seconds, and so is the list; a frame with nothing running
and no open preview asks nothing.

## 2. Design intent

- **Look, never enter.** Watching a child would take the one connection off the conversation the
  child belongs to, and the whole conversation would go down with it.
- **The child is named when it is spawned.** An empty name falls back to its first message, exactly
  as the session picker does.
- **Structure first, blocks on demand.** One answer gives every id; a screen costs one question.
- **Two witnesses, one page.** The list is a fact about the outside and is refetched; the preview is
  a window into one child's rounds.

## 3. Unresolved

1. A job's round trips are not shown here; the panel reads the child's own blocks only.
2. `Ctrl+X` stops a child and leaves it grey, but the panel learns of it on its next refresh, not
   at once.

## 4. Config fields

| key | what it decides |
|---|---|
| `config.ui.lang` | the language of the head, the figures and the hints |