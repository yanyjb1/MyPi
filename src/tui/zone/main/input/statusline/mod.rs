//! Statusline — a registry of self-updating components.
//!
//! ```text
//! +-- π > [M] model > [D] path > @ branch > $0.12 >----8%----:1M | < session --+
//! |__|  \__________________ left group: ` > ` between ________________/ \flex/  \_right_/
//! frame                                                                        frame
//! ```
//!
//! - **The row is exactly one line of exactly `width` cells.** It is row 0 of
//!   the input container (`components::input`), so its width *is* the
//!   container's width; `layout` guarantees it.
//! - **Components are stateful and push-only.** Each one caches what it shows
//!   and changes only when an event arrives ([`StatusComponent::on_event`]);
//!   rendering is a pure read. Nothing in here polls the session, the config or
//!   the theme — the theme arrives resolved ([`StatusTheme`]) and every fact
//!   arrives as a [`StatusEvent`]. The render loop is the edge detector: it
//!   turns "the session became busy" into `ActivityStarted`, "a turn ended"
//!   into `Git`/`Usage`, and so on.
//! - **Nothing is hardcoded between components.** Each declares its side, its
//!   position and its omission priority; the engine inserts the connectors.
//!
//! ## Omission priority
//!
//! Higher survives longer. Dropping removes the whole component, connector
//! included — text is never cut mid-way.
//!
//! | priority | component | on |
//! |---|---|---|
//! | [`PRIORITY_BASE`] | π, price, usage gauge | never dropped |
//! | 80 | model name | left |
//! | 60 | session name | right |
//! | 40 | working directory | left |
//! | 20 | git branch + counts | left |
//!
//! ## Adding a component
//!
//! One file under `components/`, one `Box::new(...)` line in
//! [`components::standard`]. Nothing else changes: the engine sorts by
//! `(side, order)`, the connectors follow, and the omission logic reads the
//! declared priority.

pub mod components;
mod layout;
pub mod theme;

pub use theme::{ALL_TOKENS, ColorPolicy, StatusTheme, Token};

use std::path::Path;

use ratatui::text::Line;

use crate::git::GitStatus;

/// Which group of the row a component renders in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Before the flexible group, ` > ` between neighbours.
    Left,
    /// The flexible group: takes the leftover width, owns its own connector.
    Flex,
    /// After the flexible group, every component prefixed by `< `.
    Right,
}

impl Side {
    /// Sort key: the engine visits groups in this order.
    const fn rank(self) -> u8 {
        match self {
            Side::Left => 0,
            Side::Flex => 1,
            Side::Right => 2,
        }
    }
}

/// The floor of the row: components at this priority are never dropped, and a
/// flexible component's narrowest form counts as base tier.
pub const PRIORITY_BASE: u8 = 100;

/// Tokens the *engine* uses, not a component: the frame is accent, the
/// connectors wear the separator color. Declared here so the coverage audit
/// stays honest about who consumes what.
const ENGINE_TOKENS: [Token; 2] = [Token::Accent, Token::Sep];

/// Everything a statusline component can be told. Payloads are borrowed: a
/// notification is synchronous and the component copies what it keeps.
#[derive(Debug, Clone, Copy)]
pub enum StatusEvent<'a> {
    /// One animation frame. Emitted once per render-loop pass; only the
    /// activity indicator does anything with it.
    Tick,
    /// A turn started: the activity indicator starts spinning.
    ActivityStarted,
    /// The turn ended (or died): the indicator goes back to `π`.
    ActivityStopped,
    /// `/switch` landed. The model name is passive: it changes only here.
    ModelChanged(&'a str),
    /// `/cdp` — the user's persistent workspace moved. This is the only event
    /// that re-targets the git segment.
    WorkspaceChanged(&'a Path),
    /// The model's `cd` tool migrated the session's working directory. A
    /// temporary migration: the git segment ignores it by design.
    CwdMigrated(&'a Path),
    /// `/name`, a turn start, or a resume: the session's display name changed.
    /// The sender resolves the name (explicit, or synthesized from the first
    /// user message) — a component cannot see the transcript.
    SessionRenamed(&'a str),
    /// A fresh git snapshot of the **workspace**. `None` = outside a
    /// repository (or no git): the segment hides itself.
    Git(Option<&'a GitStatus>),
    /// Cost + context size, from the cost tracker. Emitted once per frame by
    /// the render loop (the block-draw hook); the arithmetic stays where it is.
    Usage(UsageSnapshot),
}

/// What the price segment and the usage gauge display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageSnapshot {
    /// Session cumulative cost in the model's currency.
    pub total_cost: f64,
    /// Context size (numerator of the gauge).
    pub ctx_tokens: u64,
    /// Context window (denominator of the gauge).
    pub ctx_limit: u64,
    pub currency_symbol: &'static str,
    /// False when the model has no price sheet: the price segment hides.
    pub show_cost: bool,
}

/// One cell of the statusline.
///
/// Implementors hold their own state and update it in [`Self::on_event`];
/// [`Self::render`] must not mutate anything.
pub trait StatusComponent {
    /// Stable identifier, for tests and diagnostics.
    fn id(&self) -> &'static str;

    /// Which group of the row this component belongs to.
    fn side(&self) -> Side;

    /// Position inside its group, ascending = left to right.
    fn order(&self) -> u8;

    /// Omission priority: higher survives longer (see the module table).
    fn priority(&self) -> u8;

    /// Which theme tokens this component reads.
    fn colors(&self) -> ColorPolicy;

    /// Self-update. Ignore events you do not care about.
    fn on_event(&mut self, ev: &StatusEvent<'_>);

    /// Pure render.
    ///
    /// `budget` is `None` during measurement — a flexible component must then
    /// return its **narrowest** form. With `Some(cells)` the engine has
    /// allocated exactly that much: a flexible component fills it, a fixed one
    /// may ignore it.
    ///
    /// Returning an empty run hides the component (and its connector) for this
    /// frame — that is how the price segment disappears on a model with no
    /// price sheet.
    fn render(&self, t: &StatusTheme, budget: Option<usize>) -> Vec<ratatui::text::Span<'static>>;
}

/// The registry. Owned by `App`, one instance per session, alive across frames.
pub struct StatusLine {
    /// Sorted by `(side, order)` so the engine can visit it in render order.
    comps: Vec<Box<dyn StatusComponent>>,
}

impl StatusLine {
    /// The standard row. Ordering is `(side, order)`; the engine's connectors
    /// and omission logic need nothing else.
    pub fn new() -> Self {
        let mut comps = components::standard();
        comps.sort_by_key(|c| (c.side().rank(), c.order()));
        Self { comps }
    }

    /// Broadcast one event to every component.
    pub fn notify(&mut self, ev: &StatusEvent<'_>) {
        for c in &mut self.comps {
            c.on_event(ev);
        }
    }

    /// Render the row. Exactly `width` cells, always.
    pub fn render(&self, t: &StatusTheme, width: u16) -> Line<'static> {
        layout::render_line(&self.comps, t, width)
    }

    /// Every theme token the registered components declare, in [`ALL_TOKENS`]
    /// order — plus the ones the engine uses for its own chrome. The coverage
    /// audit: a token nobody declares is a token that should not exist.
    pub fn declared_tokens(&self) -> Vec<Token> {
        ALL_TOKENS
            .into_iter()
            .filter(|tok| {
                ENGINE_TOKENS.contains(tok)
                    || self.comps.iter().any(|c| match c.colors() {
                        ColorPolicy::Tokens(toks) => toks.contains(tok),
                        ColorPolicy::Fixed | ColorPolicy::Custom => false,
                    })
            })
            .collect()
    }
}

impl Default for StatusLine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::zone::main::input::statusline::theme::ALL_TOKENS;

    /// A row with every component fed one full event batch — the state a
    /// running session reaches after its first frame.
    pub(crate) fn sample() -> StatusLine {
        let mut s = StatusLine::new();
        s.notify(&StatusEvent::ModelChanged("global:model-z"));
        s.notify(&StatusEvent::WorkspaceChanged(Path::new(
            "/home/Arisha/Utility/MyPi",
        )));
        s.notify(&StatusEvent::Git(Some(&GitStatus {
            branch: Some("main".into()),
            unstaged: 0,
            staged: 0,
        })));
        s.notify(&StatusEvent::SessionRenamed("GPT5.6L(LC)"));
        s.notify(&StatusEvent::Usage(UsageSnapshot {
            total_cost: 1.2345,
            ctx_tokens: 80_000,
            ctx_limit: 1_000_000,
            currency_symbol: "¥",
            show_cost: true,
        }));
        s
    }

    pub(crate) fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    pub(crate) fn line_width(l: &Line) -> usize {
        use unicode_width::UnicodeWidthStr;
        l.spans.iter().map(|s| s.content.width()).sum()
    }

    #[test]
    fn one_row_exactly_the_asked_width() {
        let s = sample();
        let t = StatusTheme::resolve();
        for width in [40u16, 60, 80, 100, 160, 220] {
            let line = s.render(&t, width);
            let txt = text(&line);
            assert_eq!(line_width(&line), width as usize, "width @{width}: {txt}");
            assert!(txt.starts_with("+--"), "@{width}: {txt}");
            assert!(txt.ends_with("--+"), "@{width}: {txt}");
        }
    }

    /// The omission order the user specified: git first, then the working
    /// directory, then the session name, then the model name. π, the price and
    /// the gauge survive everything.
    #[test]
    fn omission_follows_the_declared_priority() {
        let s = sample();
        let t = StatusTheme::resolve();
        let at = |w: u16| text(&s.render(&t, w));

        let wide = at(200);
        assert!(wide.contains("@ main"), "{wide}");
        assert!(wide.contains("[D] Utility/MyPi"), "{wide}");
        assert!(wide.contains("GPT5.6L(LC)"), "{wide}");
        assert!(wide.contains("[M] global:model-z"), "{wide}");

        // Narrow: non-base components drop in priority order — cwd (40) first,
        // then git (55; its compact branch-only form survives one tier
        // deeper), then session (60). Model (80) is the last non-base.
        // A component "drops" when the row narrows past its threshold, so the
        // component with the HIGHER drop-width goes first.
        let mut dropped_dir = None;
        let mut dropped_git = None;
        let mut dropped_session = None;
        for w in (20..=200).rev() {
            let txt = at(w);
            if dropped_dir.is_none() && !txt.contains("[D]") {
                dropped_dir = Some(w);
            }
            if dropped_git.is_none() && !txt.contains("@ main") {
                dropped_git = Some(w);
            }
            if dropped_session.is_none() && !txt.contains("< GPT5.6L(LC)") {
                dropped_session = Some(w);
            }
        }
        let w_dir = dropped_dir.expect("the directory must eventually go");
        let w_git = dropped_git.expect("git must eventually go");
        let w_session = dropped_session.expect("session must eventually go");
        assert!(w_dir > w_git, "cwd (40) drops before git (55): {w_dir} vs {w_git}");
        assert!(w_git > w_session, "git (55) drops before session (60)");

        // The floor: π, the price and the gauge survive the narrowest row that
        // can hold the base tier (25 cells; below that the frame itself is
        // clamped, which is the one case nothing can be done about).
        for w in [30u16, 40, 50] {
            let txt = at(w);
            assert!(txt.contains("π"), "@{w}: {txt}");
            assert!(txt.contains("¥1.23"), "@{w}: {txt}");
            assert!(txt.contains(":1M"), "@{w}: {txt}");
        }
    }

    /// Extreme narrow: with only the base tier left the gauge collapses to
    /// `>pct-dashes:1M`, and the row is still exactly the requested width.
    #[test]
    fn the_gauge_collapses_to_its_narrowest_form() {
        let s = sample();
        let t = StatusTheme::resolve();
        // 25 = `+-- ` + π + ` > ` + `¥1.23` + ` ` + `>-8%:1M` + ` --+`.
        // 第一格 accent 无论如何都留着（用量不足一格时它就是"刻度起点"）。
        assert_eq!(text(&s.render(&t, 25)), "+-- π > ¥1.23 >-8%:1M --+");
        // Wide: the slack becomes dashes again.
        let wide = text(&s.render(&t, 200));
        assert!(wide.contains("---"), "{wide}");
    }

    /// Colors, per the spec: frame/model/session accent, path white, `π` muted,
    /// price gold, and everything inside a capsule carries its background.
    #[test]
    fn colors_match_the_spec() {
        let s = sample();
        let t = StatusTheme::resolve();
        let line = s.render(&t, 120);
        let find = |needle: &str| -> Option<&ratatui::text::Span<'static>> {
            line.spans.iter().find(|sp| sp.content == needle)
        };
        let capsule = t.get(Token::Capsule);

        assert_eq!(find("+").unwrap().style.fg, Some(t.get(Token::Accent)));
        let pi = find("π").unwrap();
        assert_eq!(pi.style.fg, Some(t.get(Token::Muted)));
        assert_eq!(pi.style.bg, Some(capsule));
        let model = find("[M] global:model-z").unwrap();
        assert_eq!(model.style.fg, Some(t.get(Token::Accent)));
        assert_eq!(model.style.bg, Some(capsule));
        let dir = find("[D] Utility/MyPi").unwrap();
        assert_eq!(dir.style.fg, Some(t.get(Token::Text)));
        assert_eq!(dir.style.bg, Some(capsule));
        let br = find("@ main").unwrap();
        assert_eq!(br.style.fg, Some(t.get(Token::GitClean)));
        assert_eq!(br.style.bg, Some(capsule));
        let session = find("GPT5.6L(LC)").unwrap();
        assert_eq!(session.style.fg, Some(t.get(Token::Accent)));
        assert_eq!(session.style.bg, Some(capsule));

        // Price: gold, followed by the capsule's padding cell before the gauge.
        let cost = line
            .spans
            .iter()
            .position(|sp| sp.content == "¥1.23")
            .expect("cost span");
        assert_eq!(line.spans[cost].style.fg, Some(t.get(Token::Gold)));
        let after = &line.spans[cost + 1];
        assert_eq!(
            (after.content.as_ref(), after.style.bg),
            (" ", Some(capsule)),
            "the price must be followed by one capsule padding cell"
        );

        // Gauge: the percentage shows alone, and the unused part is transparent.
        assert!(line.spans.iter().any(|sp| sp.content == "8%"));
        assert!(!line.spans.iter().any(|sp| sp.content.contains("ctx")));
        let rest = line
            .spans
            .iter()
            .find(|sp| {
                !sp.content.is_empty() && sp.content.chars().all(|c| c == '-') && sp.style.fg.is_none()
            })
            .expect("a plain transparent dash run");
        assert_eq!(rest.style.bg, None, "the unused part has no background");
    }

    /// The spinner replaces `π` while a turn is in flight and returns after.
    #[test]
    fn activity_swaps_pi_for_a_spinning_frame() {
        let t = StatusTheme::resolve();
        let mut s = sample();
        let spans = |s: &StatusLine| -> Vec<String> {
            s.render(&t, 120)
                .spans
                .iter()
                .map(|sp| sp.content.to_string())
                .collect()
        };

        let idle = spans(&s);
        assert!(idle.iter().any(|c| c == "π"), "{idle:?}");

        s.notify(&StatusEvent::ActivityStarted);
        let first = spans(&s);
        assert!(!first.iter().any(|c| c == "π"), "{first:?}");
        assert!(first.iter().any(|c| c == "|"), "{first:?}");

        s.notify(&StatusEvent::Tick);
        let second = spans(&s);
        assert_ne!(first, second, "a tick must advance the frame");

        s.notify(&StatusEvent::ActivityStopped);
        assert_eq!(spans(&s), idle, "idling again must restore the idle row");
    }

    #[test]
    fn git_counts_show_only_what_exists() {
        let t = StatusTheme::resolve();
        let mut s = sample();
        s.notify(&StatusEvent::Git(Some(&GitStatus {
            branch: Some("dev".into()),
            unstaged: 9,
            staged: 2,
        })));
        let txt = text(&s.render(&t, 200));
        assert!(txt.contains("@ dev"), "{txt}");
        assert!(txt.contains(" ?9"), "{txt}");
        assert!(txt.contains(" +2"), "{txt}");

        s.notify(&StatusEvent::Git(Some(&GitStatus {
            branch: Some("dev".into()),
            unstaged: 0,
            staged: 0,
        })));
        let clean = text(&s.render(&t, 200));
        assert!(clean.contains("@ dev"), "{clean}");
        assert!(!clean.contains(" ?"), "{clean}");
        assert!(!clean.contains(" +"), "{clean}");

        // Outside a repository the whole segment disappears.
        s.notify(&StatusEvent::Git(None));
        let none = text(&s.render(&t, 200));
        assert!(!none.contains('@'), "{none}");
    }

    /// The temporary `cd` migration moves the directory segment (and switches
    /// the marker) but never re-targets git.
    #[test]
    fn temp_migration_flips_the_marker_and_leaves_git_alone() {
        let t = StatusTheme::resolve();
        let mut s = sample();
        assert!(text(&s.render(&t, 200)).contains("[D] Utility/MyPi"));

        s.notify(&StatusEvent::CwdMigrated(Path::new("/tmp/scratch")));
        let temp = text(&s.render(&t, 200));
        assert!(temp.contains("[T] tmp/scratch"), "{temp}");
        assert!(temp.contains("@ main"), "git must keep the workspace: {temp}");

        // Only a workspace move re-targets both.
        s.notify(&StatusEvent::WorkspaceChanged(Path::new("/home/Arisha/other")));
        let moved = text(&s.render(&t, 200));
        assert!(moved.contains("[D] Arisha/other"), "{moved}");
    }

    #[test]
    fn price_hides_without_a_price_sheet() {
        let t = StatusTheme::resolve();
        let mut s = sample();
        s.notify(&StatusEvent::Usage(UsageSnapshot {
            total_cost: 0.0,
            ctx_tokens: 0,
            ctx_limit: 1000,
            currency_symbol: "$",
            show_cost: false,
        }));
        let txt = text(&s.render(&t, 200));
        assert!(!txt.contains('¥'), "{txt}");
        assert!(!txt.contains("$0.00"), "{txt}");
        assert!(txt.contains("[M] global:model-z"), "{txt}");
        // The gauge's own denominator is independent of the price.
        assert!(txt.contains(":1K"), "{txt}");
    }

    /// Every token a component declares must exist, and every existing token
    /// must be declared by someone — otherwise the token set has grown dead
    /// weight (which is exactly how the `statusLine*` theme tokens rotted).
    #[test]
    fn declared_tokens_cover_the_token_set() {
        let declared = StatusLine::new().declared_tokens();
        for tok in ALL_TOKENS {
            assert!(
                declared.contains(&tok),
                "token {tok:?} is declared by nobody — remove it or use it"
            );
        }
    }

    // ---- the ColorPolicy interface ----

    struct FixedDot;
    impl StatusComponent for FixedDot {
        fn id(&self) -> &'static str {
            "fixed-dot"
        }
        fn side(&self) -> Side {
            Side::Right
        }
        fn order(&self) -> u8 {
            9
        }
        fn priority(&self) -> u8 {
            10
        }
        fn colors(&self) -> ColorPolicy {
            ColorPolicy::Fixed
        }
        fn on_event(&mut self, _ev: &StatusEvent<'_>) {}
        fn render(&self, _t: &StatusTheme, _b: Option<usize>) -> Vec<ratatui::text::Span<'static>> {
            vec![ratatui::text::Span::styled(
                "dot",
                ratatui::style::Style::new().fg(ratatui::style::Color::Red),
            )]
        }
    }

    struct CustomDot;
    impl StatusComponent for CustomDot {
        fn id(&self) -> &'static str {
            "custom-dot"
        }
        fn side(&self) -> Side {
            Side::Right
        }
        fn order(&self) -> u8 {
            9
        }
        fn priority(&self) -> u8 {
            10
        }
        fn colors(&self) -> ColorPolicy {
            ColorPolicy::Custom
        }
        fn on_event(&mut self, _ev: &StatusEvent<'_>) {}
        fn render(&self, t: &StatusTheme, _b: Option<usize>) -> Vec<ratatui::text::Span<'static>> {
            // Reads the resolved value and applies it its own way.
            vec![t.capsule("x", t.get(Token::Gold))]
        }
    }

    /// `Fixed` means exactly that: the theme is irrelevant to this component.
    /// `Custom` reads the resolved values and applies them its own way. Both
    /// declare no tokens, so neither enters the coverage union.
    #[test]
    fn color_policies_are_honoured() {
        // The component's *own* span (the engine's chrome legitimately follows
        // the theme, so the whole row is not the right unit here).
        let own = |l: &Line, glyph: &str| -> ratatui::style::Style {
            l.spans
                .iter()
                .find(|s| s.content == glyph)
                .expect("the component's span")
                .style
        };
        // Two snapshots that differ in every slot.
        let a = StatusTheme::from_parts([ratatui::style::Color::Red; ALL_TOKENS.len()]);
        let b = StatusTheme::from_parts([ratatui::style::Color::Blue; ALL_TOKENS.len()]);

        let fixed = StatusLine {
            comps: vec![Box::new(FixedDot)],
        };
        assert_eq!(
            own(&fixed.render(&a, 40), "dot"),
            own(&fixed.render(&b, 40), "dot"),
            "a Fixed component must not follow the theme"
        );
        assert_eq!(
            fixed.declared_tokens(),
            vec![Token::Accent, Token::Sep],
            "only the engine's own chrome is declared here"
        );

        let custom = StatusLine {
            comps: vec![Box::new(CustomDot)],
        };
        assert_ne!(
            own(&custom.render(&a, 40), "x"),
            own(&custom.render(&b, 40), "x"),
            "a Custom component reads the resolved values"
        );
        assert_eq!(custom.declared_tokens(), vec![Token::Accent, Token::Sep]);
    }
}
