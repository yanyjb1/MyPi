# history — architecture

What is here, what each file holds, who calls whom. Current state only.

## Files

| path | holds |
|---|---|
| `mod.rs` | the module list and the re-export |
| `shape.rs` | `Block`, `Kind`, `Usage`: one decoded block row |
| `local.rs` | `Local`: the numbered slots; `Level`, `Life`, `SlotKind` |
| `segment.rs` | `split`: one block's text into segments |
| `line.rs` | `Row`, `Run`, `Ink`, `Lead`: a finished row and its color slots |
| `wrap.rs` | natural line → width rows |
| `fold.rs` | a long run of rows → head, placeholder, tail |
| `paint/` | every block kind to natural rows |
| `paint/prose.rs` | text segments, a whole markdown text, the thinking chain |
| `paint/markdown.rs` | headings, lists, quotes, inline emphasis |
| `paint/code.rs` | fenced code: syntect state, one line at a time |
| `paint/table.rs` | tables, laid out at the width |
| `paint/card/` | tool cards: the renderer interface and one file per tool family |
| `paint/user.rs` | the user card |
| `paint/todo.rs` | the pinned task list |
| `paint/notice.rs` | a notice slot |
| `paint/opts.rs` | `Opts`: fold switches, placeholder, cwd, budget |
| `cache/text.rs` | segment natural rows per block |
| `cache/rows.rs` | width rows per block, with the natural-line marks |
| `cache/store.rs` | the two layers, their keys, the block classes |
| `live.rs` | the growing tail and its per-block segment cache |
| `view.rs` | the sight line, the seam, the window |
| `zone.rs` | the zone: types, keys, the `SubZone` contract |
| `feed.rs` | frames in: pieces, closed rounds, notices, config, structure |
| `ask.rs` | which blocks to fetch |
| `render.rs` | one frame: cache, window, accent at the exit |

## Who calls whom

- `zone::main::zone` calls `history::HistoryZone::render_rows(width, accent)` and the `SubZone` methods.
- `history::render` drives `view::compose`, which asks the cache for one slot's rows at a time.
- `history::paint::card` calls `paint::prose::inline` for the prose above a tool call, and `fold::fold` for its sections.
- `paint::code` calls nothing above it; it owns the syntect tables.

## Must stay in sync

1. `line::Lead` and `wrap`: a folded row must become `Wrapped`, or the reader loses the marker.
2. `cache::store::Class` and `render::block_rows`: a card painted as segments loses its frame.
3. `view::Spot` and `cache::rows::Entry::row_of` / `spot_of`: the seam survives a resize only through the pair.
4. `shape::Kind` and `paint`: a new block kind needs a painter.
5. `feed.rs` and the config keys in `zone.rs`: one key read, one key asked for.

## Not wired

1. The tool-specific stream animations: the running card is a static card.
2. `next` slots render nothing and break no separator; that is intended, not a gap.
