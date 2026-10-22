# history — architecture

Where the history zone sits, what each file holds, and what it depends on. Current state only.

## Files

| file | holds | outside |
|---|---|---|
| `mod.rs` | the module list | private |
| `zone.rs` | `HistoryZone`, `Config`, `CLAIMS`, the frame pass | private to `Main` |
| `ask.rs` | `Ask` — init, jump, fetch, settled | private |
| `local.rs` | `Local`, `Slot`, `SlotKind` — the append-only section | private |
| `view.rs` | `Viewport`, `Anchor`, `Dir`, `Ahead` — sight line and layout walk | private |
| `todo.rs` | `TaskList` — the server's start plus increments | private |
| `resize.rs` | `ResizePlan`, `plan` — visible vs lazy numbers | private |
| `product/` | `Product`, `Segment`, `CutPoint`, `CutKind` | private |
| `layout/` | `layout`, `visible::apply`, `lazy::nums` | private |
| `cache/` | `BlockCache`, `Item` — products, rows, heights, versions | private |
| `render/` | painting: block, cards, text, theme | private |

## What it exposes

Through `SubZone`:

- `claims`, `deliver_event`, `deliver_action`
- `attach`, `detach`
- `request_height`, `assign`, `on_height_changed`
- `take_requests`

To `Main`:

- `set_config(&Config)`
- `warm(&View)`
- `notice(text, level, life)`
- `reset()`
- `focus(turn, block)`
- `render_rows(view, width, theme)`
- `take_out() -> Vec<Ask>`

`Config` fields: `focus_line`, `focus_margin`, `rest_ms`, `overscan`, `on_thinking`,
`unfold_thinking`, `unfold_tool`, `todo_fold`.

## What it calls

- `ui::session::View`: `chain`, `blocks`, `todo`, `tail`, `turn`, `busy`, `generation`, `cwd`,
  `notices`.
- `ui::session::{BlockRef, Level, Life}`.
- `server::storage::BlockKind`.
- `server::agent::tools::TodoPhase` and the todo state.
- `server::config::read::config::TodoFold`.
- `theme::theme_epoch`.
- `render::text::highlight::warm_in_order`.
- `history::render::cards::{chat, todo, notice}`.
- `text` for width and wrapping.

## Who calls it

`MainZone` is the only caller:

- `arbitrate` → `assign`
- `settle` / `broadcast` → `on_height_changed`
- `render` → `render_rows`
- `dispatch` → `deliver_action`
- `deliver_event` → `deliver_event`
- `attach` / `detach`
- `take_requests` → `take_out`
- `take_over` → `focus`

## What it does not know

The app, the input zone, the reserved zone, the picker, the tree, the socket, the database, and
the agent loop. It never sends anything itself; every Ask leaves as a `Request::View`.

## Must stay in sync

1. `Config` fields with the config keys in `README.md`; only three keys are read today.
2. `Ask` with the loop's landing site; a variant without one is silent work.
3. `Local`'s four slot kinds with the layout walk's height and separator rules.
4. `Item`'s number/block/version with the cache's `ensure` and `sync`.
5. The two copies in `cache/` with the switches in `HistoryZone`.
6. `Product` and `layout`: a new product kind needs a wrap and a cut rule.
7. `resize::plan` and `layout::visible`: the visible list is applied every frame; the lazy list is
   not consumed yet.
8. The cache's invalidation keys with `theme_epoch`, `generation`, `tail.version`, `width`, and
   `cwd`.

## Not wired

1. `Request::View` has no landing site; the loop does not exist.
2. `Session` has no `init` and no `Call::Init`; `Ask::Init` cannot be answered as designed.
3. `layout::lazy::nums` has no caller.
4. `layout::layout` ignores its `seam` argument.
5. `HistoryZone::notice` has no caller outside the zone.
6. `ui` is not in the build.
