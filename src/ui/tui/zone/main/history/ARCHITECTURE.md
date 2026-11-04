# history — architecture

What each file holds, who calls it, and what it depends on. Current state only.

## Files

| path | holds | outside |
|---|---|---|
| `mod.rs` | the module list | private |
| `zone.rs` | `HistoryZone`, its claims, the `SubZone` implementation | public |
| `display.rs` | `Display`, `Dir`, the window, the scroll, the focus, the align | private |
| `prefetch.rs` | `Prefetch`, `Laid`, `Origin`, the walk outwards, the piece budget | private |
| `local.rs` | `Local`, `Slot`, `SlotKind`, `Level`, `Life` | private |
| `layout/` | `segment` (source to segments), `line` (`Row`, `Run`, `Ink`, `Lead`), `wrap`, `fold` | private |
| `product/` | `Products` and the two product kinds: `markdown` (pieces), `card` (rows), `todo` (the checklist) | private |
| `wire/` | `mod` (frame dispatch, busy and idle), `decode` (block rows), `ask` (blocks on demand), `live` (the growing tail), `transfer` (the handover) | private |
| `paint/` | how one block is drawn: prose, code, table, notice, user, cards | private |
| `paint/card/` | `frame` (the renderer contract, dispatch, the card frame), one file per tool family: `bash`, `edit`, `file`, `todo`, `web`, `alias`, `memo`, `context`, `kairos`, `mcp`, `task`, `draft`, `fallback`, and `tree` for the branch rows | private |

## What it talks to

- **Calls**: `ui::tui::zone` for the contract, `ui::tui::theme` for the fixed palette and the accent,
  `ui::tui::TodoFold`, `ui::tui::action`.
- **Asks the session for**: `call init`, `call blocks`, `call config`, `call usage`, `call todo` and
  the structural window, all through `Request` values its host drains.
- **Does not know**: the app, the socket, the database, the agent loop, the picker.

## Inside

```text
HistoryZone
 ├── blocks: BTreeMap<i64, Block>        ── what the wire sent
 ├── local: Local                        ── the numbered slots
 ├── live: Live                          ── the tail still growing
 ├── products: Products                  ── block → pieces, `cacheMargin` of them
 ├── prep: Prefetch                      ── the laid rows around the sight line
 ├── disp: Display                       ── where the window starts
 ├── todo_panel: TodoPanel               ── the checklist's rows
 └── thinking / tool / fold / scroll_height / placeholder / cwd / usage
```

- `wire::mod::read_frame` decodes one frame; a closed round hands the tail over and asks for the
  missing blocks.
- `display::render_rows` refolds when the width moved, takes the window from `prefetch`, and puts
  the checklist under it.
- `prefetch` walks one piece at a time: `extend_newer` for the growing end (which re-lays the live
  tail in place), `extend_older` for the past.
- `product::Products` is the only place a block is turned into pieces; it keeps `cacheMargin` blocks
  and drops the oldest admission first.

## Must stay in sync

1. `wire::decode::Kind` and `paint/` plus `product/`: a new block kind needs a product and a painter.
2. `product::Piece` and `prefetch`: a new piece kind needs a way to be laid.
3. `layout::segment::Kind` and `product::markdown`: a new segment kind needs a painter.
4. The config key names and `zone::CONFIG_KEYS` plus `wire::mod::read_config`.
5. `local::SlotKind` and `prefetch::piece_count` / `layer_piece`: a new slot kind needs both.
6. A tool's memo shape and its `paint/card/` file: the renderer draws what the memo carries, or the
   result text when the tool hands back no memo.

## Not wired

1. A width change reaches the history zone through `assign`; the `local / resized` frame is not read.
2. The main zone owns the rows; this zone answers `main::pane::Pane`.
