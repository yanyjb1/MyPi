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
| `local/` | `section` (`Local`, `Idx`), `slot` (`Slot`, `SlotKind`, `Held`, `Level`, `Life`), `content` (`Block`, `Kind`), `mutable`, `immutable`, `handover`, `fetch` | private |
| `product/` | `products` (`Product`, `Shape`, the derivation), `segment` (source to segments), `text`, `natural`, `fold`, `opts`, `notice`, `user`, `table`, `todo`, `pin`, `card` | private |
| `product/pin/` | `esc` (the running label's row), `sweep` (the accent band over that label) | private |
| `product/card/` | `frame` (dispatch, the card frame, the text and argument helpers), `tool` (the renderer contract and the family table), `state` (the three phases and their colors), `partial` (fields of arguments still arriving), one file per tool family | private |
| `prefetch/` | `run` (the laid segments), `range` (the start and the budget), `segment` (`SegId`, `Laid`), `trim`, `walk`, `color` | private |

## What it talks to

- **Calls**: `ui::tui::zone` for the contract (`Frame`, `Request`, `Shout`, `Turns`, `Claim`),
  `ui::tui::theme` for the fixed palette and the accent, `ui::tui::TodoFold`,
  `ui::tui::action` for keys and wheel, `ui::tui::text` for display width, byte counts and
  durations.
- **Asks the session for**: `call init`, `call blocks`, `call config`, `call usage`,
  and the structural window, all through `Request` values its host drains.
- **Does not know**: the app, the socket, the database, the agent loop, the picker.

## Inside

```text
HistoryZone
 ├── local: Local               ── two regions: positions, each with its content and product
 ├── run: Run                   ── the laid segments, in render order
 ├── range: Range               ── `Latest` or one fixed place, plus the screen height
 ├── margin: usize              ── blocks that keep their content and product
 ├── dirty: BTreeSet<Idx>       ── positions a piece or a block touched since the last tick
 ├── missing: Vec<i64>          ── blocks the last draw lacked
 ├── todo_panel: TodoPanel      ── the checklist's rows
 ├── pin: Vec<Pin>              ── what is still running, in arrival order
 ├── tick: usize                ── the phase the spinner reads
 ├── busy: bool                 ── the statusline's busy glyph
 ├── rev: u64                   ── what the app compares to decide on a draw
 └── thinking / tool / fold / scroll_height / focus_line / placeholder / cwd / pending
```

- `wire::read_frame` decodes one frame and never lays: a piece appends to the running entry and marks
  that entry's position (and the card position it draws), a block binds to the slot its id owns.
  `call init` lays the structure out before the blocks, so every id has its slot before its content
  arrives. A receipt either hands the running entries their numbers or, when the count does not line
  up, reserves its own positions and binds the fetched rows to them.
- `zone::tick` applies the dirty set once: those positions lose their laid rows and their product,
  the content budget is settled against the window the last draw kept, and the blocks the last draw
  lacked leave as `Request::Blocks`. `zone::render_rows` rebuilds the pin, spends the window, and
  trims the run; it changes nothing.
- `Local::prepare` builds a position's product on first use and keeps it on the block; `Local::shape`
  hands the run what that product is. The run lays only segments it does not hold.
- `Local::evict` drops content past `cacheMargin`, oldest admission first, skipping positions the run
  holds, the ranges asked for, and the ranges a close is waiting on. One exchange is kept or dropped
  as one, so a card never loses half its rows. A position whose content left draws `…` and asks
  again through `Request::Blocks`.

## Must stay in sync

1. `local::content::Kind` and `product::products::of`: a new block kind needs a product.
2. `product::segment::Kind` and `prefetch::color`: a new segment kind needs a painter.
3. The config key names and `zone::CONFIG_KEYS` plus `wire::read_config`.
4. `SlotKind` and `Local::shape`: a new slot kind needs a shape.
5. A tool's memo shape and its `product/card/` file: the renderer draws what the memo carries, or
   the result text when the tool hands back no memo.
6. `Range::scroll` and the wheel bindings in `zone::deliver_action`.
7. `Local::note_pair`, `Local::card_at` and every `product::of` that takes a pair: the card is drawn
   from the result its call names, so a new pairing source must reach all of them.
8. The wire's `stream / arguments` frame, its `name` field, `Mutable::arguments` and the family
   renderer's `partial` reads: a new streamed field means those four move together.
9. `tool_end`'s `truncated` field and `Block.truncated`: the relay writes it, the wire decodes it,
   and `card::frame::done` draws it.
10. `Start::Pin` and `focus::anchor_slot`: a window top inside the pin is "latest" for every
    position-based command.
11. `zone::pin_rows` and the `Pin` lifecycle in `wire`: a new kind of running work must start and
    end there, or the ESC row sticks.
12. A product's inputs and `Local::drop_products`: a new option, a new width-dependent renderer or a
    new block field means the invalidation points must know about it.
13. An ingest path and `HistoryZone::mark`: whatever changes a position's content must mark it, or
    the screen keeps the rows it drew before.
14. A receipt and the rows fetched for it: the ids come from the receipt, the content from
    `call blocks`, and the positions from the receipt alone. Nothing gives a position to content
    that did not come from a receipt.

## Not wired

1. A width change reaches the history zone through `assign`; the `local / resized` frame is not read.
2. The main zone owns the rows; this zone answers `main::pane::Pane`.
