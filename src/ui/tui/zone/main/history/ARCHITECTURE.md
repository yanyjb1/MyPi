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
| `local/` | `section` (`Local`, `Idx`, `Pair`), `slot` (`Slot`, `SlotKind`, `Level`, `Life`), `content` (`Block`, `Kind`), `mutable`, `immutable`, `handover`, `fetch` | private |
| `product/` | `products` (the cache), `segment` (source to segments), `text`, `natural`, `fold`, `opts`, `notice`, `user`, `table`, `todo`, `pin`, `card` | private |
| `product/pin/` | `esc` (the running label's row), `sweep` (the accent band over that label) | private |
| `product/card/` | `frame` (dispatch, the card frame, the text and argument helpers), `tool` (the renderer contract and the family table), `state` (the three phases and their colors), `partial` (fields of arguments still arriving), one file per tool family | private |
| `prefetch/` | `run` (the laid segments), `range` (the start and the budget), `segment` (`SegId`, `Laid`), `trim`, `walk`, `color` | private |

## What it talks to

- **Calls**: `ui::tui::zone` for the contract (`Frame`, `Request`, `Shout`, `Turns`, `Claim`),
  `ui::tui::theme` for the fixed palette and the accent, `ui::tui::TodoFold`,
  `ui::tui::action` for keys and wheel, `ui::tui::text` for display width, byte counts and
  durations.
- **Asks the session for**: `call init`, `call blocks`, `call config`, `call usage`, `call todo`,
  and the structural window, all through `Request` values its host drains.
- **Does not know**: the app, the socket, the database, the agent loop, the picker.

## Inside

```text
HistoryZone
 ├── local: Local               ── positions and the blocks behind them
 ├── products: Products         ── block id plus its pair to segments or rows, `cacheMargin` of them
 ├── run: Run                   ── the laid segments, in render order
 ├── range: Range               ── `Latest` or one fixed place, plus the screen height
 ├── todo_panel: TodoPanel      ── the checklist's rows
 ├── pin: Vec<Pin>              ── what is still running, in arrival order
 ├── tick: usize                ── the phase the spinner reads
 └── thinking / tool / fold / scroll_height / focus_line / placeholder / cwd / pending
```

- `wire::read_frame` decodes one frame. A piece appends to the running entry and re-lays that
  entry's last segment, and the other half of its tool exchange with it. A round close hands the
  entries over and the run starts again; when the receipt does not line up, `pending` holds its
  ranges, the running turn stays on screen, and its rows finish the handover when they land.
- `zone::render_rows` builds the pin (the checklist rows, then the ESC row while something runs),
  hands it to the run, spends the screen from `range`, and reports any block whose content is
  missing.
- `Range::rows` walks forward from the start, laying segments as it goes; `Range::scroll` moves the
  start and reports missing content along the way; the follow start fills towards the old side and
  reports the same way.
- `Run` keeps whole segments only: `place` lays one, `touch` re-lays a growing entry's tail,
  `forget` drops one position's rows, `trim` keeps every segment the screen touches and drops the
  rest. `set_pin` replaces the reserved tail rows; `len` and `row` count them, `body_len` and
  `pin_len` split them.
- `Products` is the only place a block becomes segments or rows; a key is a block id plus the
  result id it pairs with, and it keeps `cacheMargin` of them, oldest admission first.
- `Local::result_of` is the one pairing rule: a tool request finds its result by call id, whichever
  store holds the position. Nothing pairs by position.

## Must stay in sync

1. `local::content::Kind` and `product::products::of`: a new block kind needs a product.
2. `product::segment::Kind` and `prefetch::color`: a new segment kind needs a painter.
3. The config key names and `zone::CONFIG_KEYS` plus `wire::read_config`.
4. `SlotKind` and `Run::shape`: a new slot kind needs a shape.
5. A tool's memo shape and its `product/card/` file: the renderer draws what the memo carries, or
   the result text when the tool hands back no memo.
6. `Range::scroll` and the wheel bindings in `zone::deliver_action`.
7. `Local::result_of`, `Local::card_at`, `Products`' key and every `product::of` that takes a pair:
   the card is drawn from the result its call names, so a new pairing source must reach all of them.
8. The wire's `stream / arguments` frame, its `name` field, `Mutable::arguments` and the family
   renderer's `partial` reads: a new streamed field means those four move together.
9. `tool_end`'s `truncated` field and `Block.truncated`: the relay writes it, the wire decodes it,
   and `card::frame::done` draws it.
10. `Start::Pin` and `focus::anchor_slot`: a window top inside the pin is "latest" for every
    position-based command.
11. `zone::pin_rows` and the `Pin` lifecycle in `wire`: a new kind of running work must start and
    end there, or the ESC row sticks.

## Not wired

1. A width change reaches the history zone through `assign`; the `local / resized` frame is not read.
2. The main zone owns the rows; this zone answers `main::pane::Pane`.
