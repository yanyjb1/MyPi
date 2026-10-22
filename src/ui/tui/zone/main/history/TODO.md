# history — TODO

- Land `Ask::Init` through a future `Session::init`; `Session::enter` still uses `Call::Tail`.
- Consume the lazy half of `ResizePlan`; `layout::lazy::nums` has no caller.
- Make `layout::layout` use the seam to choose a cut point; today it keeps only the anchor.
- Define config keys for `focus_line`, `rest_ms`, `on_thinking`, `unfold_thinking`, and
  `unfold_tool`.
- Revisit width-dependent card and table products; a width change can only re-wrap what was
  generated at the old width.
- Give `HistoryZone::notice` a caller in the frame loop.
- Decide terminal-too-small behavior; the current arithmetic only clamps.
