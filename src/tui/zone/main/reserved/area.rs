//! Bottom reserved area — a **claim registry**.
//!
//! The strip owns exactly one piece of state: who currently holds it and how
//! many rows were frozen when the claim opened. A service (completion popup
//! today; others later) registers a [`Claim`] and borrows the strip for its
//! lifetime; it never negotiates height again — the rows frozen on the first
//! frame of a claim stay fixed until the holder releases, even if its content
//! would later need fewer or more rows.
//!
//! Height rule (the single authority in this file):
//! - first frame of a claim: `want().min(max()).min(term_h)` frozen;
//! - every later frame: the frozen value, untouched;
//! - the only override is the terminal itself (rows must fit `term_h`);
//! - idle (no holder): 1 blank row.
//!
//! Key ownership stays with the input zone. A holder is a **borrower**: it may
//! observe and optionally consume keys via [`Claim::on_key`], and returns them
//! implicitly by releasing. The strip never owns key semantics.

use ratatui::text::Line;

use crate::tui::zone::main::input::semantics::Action;
use crate::tui::theme::Palette;

/// Default height cap when a service does not declare its own.
pub const DEFAULT_MAX: usize = 6;

/// Idle height: one blank row, so the layout never jumps on claim/release.
pub const IDLE_ROWS: u16 = 1;

/// One service's borrowing of the reserved strip.
///
/// Implementors own their content and state; the strip only arbitrates.
/// All methods take `&self`/`&mut self` as appropriate and are called only
/// from the frame path (single-threaded TUI).
pub trait Claim {
    /// Stable identity, used for `debug_assert!` on double-claims and for
    /// idempotent release.
    fn id(&self) -> &'static str;

    /// Rows wanted right now. `None` = not claiming this frame (the strip is
    /// idle). Read fresh every frame — the strip never caches a stale want.
    /// Only the **first** claim frame's value is frozen into the height.
    fn want(&self) -> Option<usize>;

    /// This service's own row ceiling. May differ from [`DEFAULT_MAX`].
    fn max(&self) -> usize {
        DEFAULT_MAX
    }

    /// The keys this service explicitly accepts, borrowed for its claim's
    /// lifetime. **Mandatory self-declaration** — a borrower never gets
    /// keys it did not ask for. An action outside this list never reaches
    /// `on_key`: the input zone offers only actions its hang state lends
    /// AND the holder declared; undeclared offers are silently returned.
    fn accepts(&self) -> &'static [Action];

    /// Key borrow: the input zone forwards every accepted [`Action`] here
    /// while this claim is active. Return `true` if consumed (the action
    /// stops there), `false` to hand it back to the input zone untouched —
    /// a borrower may just observe. Never grants permanent ownership:
    /// release returns all keys.
    fn on_key(&mut self, _action: &Action) -> bool {
        false
    }

    /// Draw into `rows` rows at `term_w` width. Called once per frame while
    /// this claim is active; `rows` is the frozen height, clipped to `term_h`.
    fn draw(&self, term_w: u16, rows: usize, p: &Palette) -> Vec<Line<'static>>;
}

/// The strip's own state: the active claim, its identity, and the frozen
/// height. Nothing else — content lives in the claimants.
#[derive(Default)]
pub struct ReservedArea {
    holder: Option<(&'static str, usize)>,
}

impl ReservedArea {
    /// One arbitration point per frame. Called by the frame path with every
    /// registered claim.
    ///
    /// - No claim wants rows -> idle blank row (and any stale holder is
    ///   forgotten: claims self-expire by going silent).
    /// - Exactly one wants -> first frame freezes `want().min(max())` (the
    ///   terminal clip happens in [`Self::rows`], which knows `term_h`).
    /// - Two want in the same frame -> `debug_assert!`; the first registrant
    ///   wins in release builds (no queue, no priority).
    pub fn resolve(&mut self, claims: &mut [&mut dyn Claim]) {
        let mut taker: Option<(&'static str, Option<usize>)> = None;
        for c in claims.iter() {
            if let Some(w) = c.want() {
                if let Some((prev, _)) = taker {
                    debug_assert!(
                        false,
                        "保留区被重复占用: {} 与 {} 同帧要位",
                        prev, c.id()
                    );
                    continue;
                }
                taker = Some((c.id(), Some(w)));
            }
        }
        match taker {
            None => self.holder = None,
            Some((id, Some(want))) => {
                // Fresh claim freezes; an ongoing claim keeps its frozen rows
                // regardless of what `want` reports now.
                let rows = match self.holder {
                    Some((id0, frozen)) if id0 == id => frozen,
                    _ => {
                        let maxed = claims
                            .iter()
                            .find(|c| c.id() == id)
                            .map(|c| c.max())
                            .unwrap_or(DEFAULT_MAX);
                        want.max(1).min(maxed)
                    }
                };
                self.holder = Some((id, rows));
            }
            Some((_, None)) => unreachable!(),
        }
    }

    /// Rows the strip occupies this frame. The only height exit in the
    /// project: layout takes this value and must not re-clamp it.
    pub fn rows(&self, term_h: u16) -> u16 {
        match self.holder {
            None => IDLE_ROWS,
            Some((_, frozen)) => (frozen as u16).min(term_h.max(1)),
        }
    }

    /// 借键投递：输入区 hang 时把翻译好的动作送来，活动持有者按自己的
    /// `accepts()` 声明自取；未声明的动作退回（false = 输入区自己消化）。
    /// 没有持有者时同样退回。Zone 只投递，不认识任何键。
    pub fn offer(&mut self, action: &Action, claims: &mut [&mut dyn Claim]) -> bool {
        let Some((id, _)) = self.holder else {
            return false;
        };
        let Some(c) = claims.iter_mut().find(|c| c.id() == id) else {
            return false;
        };
        c.on_key(action)
    }

    /// Who holds the strip now (identity string), for the draw dispatch.
    pub fn holder(&self) -> Option<&'static str> {
        self.holder.map(|(id, _)| id)
    }

    /// Explicit release by identity. Idempotent; a non-holder cannot drop
    /// someone else's claim.
    pub fn release(&mut self, id: &str) {
        if self.holder.as_ref().is_some_and(|(h, _)| *h == id) {
            self.holder = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;
    use std::cell::Cell;

    // A scripted claimant for tests.
    struct Fake {
        id: &'static str,
        want: Cell<Option<usize>>,
        keys_seen: Cell<usize>,
        consumed: Cell<bool>,
    }
    impl Fake {
        fn new(id: &'static str, want: Option<usize>) -> Self {
            Self {
                id,
                want: Cell::new(want),
                keys_seen: Cell::new(0),
                consumed: Cell::new(false),
            }
        }
    }
    impl Claim for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn want(&self) -> Option<usize> {
            self.want.get()
        }
        fn accepts(&self) -> &'static [Action] {
            &[]
        }
        fn on_key(&mut self, _a: &Action) -> bool {
            self.keys_seen.set(self.keys_seen.get() + 1);
            self.consumed.get()
        }
        fn draw(&self, _w: u16, rows: usize, _p: &Palette) -> Vec<Line<'static>> {
            (0..rows)
                .map(|i| Line::from(Span::raw(format!("{}-{}", self.id, i))))
                .collect()
        }
    }

    #[test]
    fn idle_is_one_blank_row() {
        let r = ReservedArea::default();
        assert_eq!(r.rows(24), 1);
        assert_eq!(r.holder(), None);
    }

    #[test]
    fn first_claim_freezes_height() {
        let mut f = Fake::new("t", Some(4));
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(24), 4);
        // Want shrinks and grows later — frozen rows never move.
        f.want.set(Some(2));
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(24), 4, "冻结后想变少也不动");
        f.want.set(Some(6));
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(24), 4, "冻结后想变多也不动");
    }

    #[test]
    fn claim_clamped_by_service_max_and_terminal() {
        let mut f = Fake::new("t", Some(20));
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]); // max() default 6
        assert_eq!(r.rows(24), 6, "超过服务 max 时按 max 冻结");
        r.release("t");
        f.want.set(Some(50));
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(3), 3, "冻结行数装不进终端时按终端裁");
    }

    #[test]
    fn silence_expires_the_claim() {
        let mut f = Fake::new("t", Some(3));
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(24), 3);
        f.want.set(None);
        r.resolve(&mut [&mut f]);
        assert_eq!(r.rows(24), 1, "占用者沉默 -> 回到空闲空行");
        assert_eq!(r.holder(), None);
    }

    #[test]
    fn release_is_idempotent_and_identity_scoped() {
        let mut f = Fake::new("t", Some(3));
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]);
        r.release("other");
        assert_eq!(r.rows(24), 3, "别人不能替持有者放手");
        r.release("t");
        assert_eq!(r.rows(24), 1);
        r.release("t");
        assert_eq!(r.rows(24), 1);
    }

    #[test]
    fn offer_only_while_held() {
        let mut f = Fake::new("t", Some(2));
        f.consumed.set(true);
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]);
        assert!(r.offer(&Action::Complete, &mut [&mut f]));
        assert_eq!(f.keys_seen.get(), 1);
        f.want.set(None);
        r.resolve(&mut [&mut f]);
        assert!(!r.offer(&Action::Complete, &mut [&mut f]));
        assert_eq!(f.keys_seen.get(), 1, "释放后键不再借出");
    }

    #[test]
    fn offer_passes_through_when_not_consumed() {
        let mut f = Fake::new("t", Some(2));
        f.consumed.set(false);
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut f]);
        assert!(!r.offer(&Action::Submit, &mut [&mut f]));
        assert_eq!(f.keys_seen.get(), 1, "旁观也算借到");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "保留区被重复占用")]
    fn double_claim_asserts() {
        let mut a = Fake::new("a", Some(2));
        let mut b = Fake::new("b", Some(2));
        let mut r = ReservedArea::default();
        r.resolve(&mut [&mut a, &mut b]);
    }
}
