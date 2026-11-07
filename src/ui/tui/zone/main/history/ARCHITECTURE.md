# history — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list | private |
| `zone.rs` | `HistoryZone`, its claims, the `SubZone` implementation, the `Pane` implementation | public |
| `wire.rs` | frame dispatch: content to the local section, the checklist to its product, config to each part | private |
| `row.rs` | `Row`, `Run`, `Ink`, `Lead`, and how a color slot becomes a terminal style | private |
| `markdown.rs` | markdown to runs: headings, lists, quotes, inline emphasis | private |
| `display.rs` | laid rows to screen lines | private |
| `local/` | `section` (`Local`, `Idx`), `slot` (`Slot`, `SlotKind`, `Level`, `Life`), `content` (`Block`, `Kind`), `mutable`, `immutable`, `handover`, `fetch` | private |
| `product/` | `products` (the cache), `segment` (source to segments), `text`, `natural`, `fold`, `opts`, `notice`, `user`, `table`, `todo`, `card` | private |
| `product/card/` | `frame` (dispatch, the card frame, the renderer contract), one file per tool family | private |
| `prefetch/` | `run` (the laid segments), `range` (the start and the budget), `segment` (`SegId`, `Laid`), `trim`, `walk`, `color` | private |

## What it talks to

- **Calls**: `ui::tui::zone` for the contract (`Frame`, `Request`, `Shout`, `Turns`, `Claim`),
  `ui::tui::theme` for the fixed palette and the accent, `ui::tui::TodoFold`,
  `ui::tui::action` for keys and wheel, `ui::tui::text` for display width.
- **Asks the session for**: `call init`, `call blocks`, `call config`, `call usage`, `call todo`,
  and the structural window, all through `Request` values its host drains.
- **Does not know**: the app, the socket, the database, the agent loop, the picker.

## Inside

```text
HistoryZone
 ├── local: Local               ── positions and the blocks behind them
 ├── products: Products         ── block id to segments or rows, `cacheMargin` of them
 ├── run: Run                   ── the laid segments, in render order
 ├── range: Range               ── `Latest` or one fixed place, plus the screen height
 ├── todo_panel: TodoPanel      ── the checklist's rows
 └── thinking / tool / fold / scroll_height / focus_line / placeholder / cwd
```

- `wire::read_frame` decodes one frame. A piece appends to the running entry and re-lays only that
  entry's last segment. A round close hands the entries over; the run starts again.
- `zone::render_rows` spends the budget from `range`, draws the checklist under the transcript, and
  reports any block whose content is missing.
- `Range::rows` walks forward from the start, laying segments as it goes; `Range::scroll` moves the
  start and reports missing content along the way.
- `Run` keeps whole segments only: `place` lays one, `touch` re-lays a growing entry's tail,
  `forget` drops one position's rows, `trim` keeps what the screen and its slack cover.
- `Products` is the only place a block becomes segments or rows; it keeps `cacheMargin` blocks and
  drops the oldest admission first.

## Must stay in sync

1. `local::content::Kind` and `product::products::of`: a new block kind needs a product.
2. `product::segment::Kind` and `prefetch::color`: a new segment kind needs a painter.
3. The config key names and `zone::CONFIG_KEYS` plus `wire::read_config`.
4. `SlotKind` and `Run::shape`: a new slot kind needs a shape.
5. A tool's memo shape and its `product/card/` file: the renderer draws what the memo carries, or
   the result text when the tool hands back no memo.
6. `Range::scroll` and the wheel bindings in `zone::deliver_action`.

## Not wired

1. A width change reaches the history zone through `assign`; the `local / resized` frame is not read.
2. The main zone owns the rows; this zone answers `main::pane::Pane`.
