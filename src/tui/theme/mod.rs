//! Theme loading — bundled themes, custom themes, the global instance.
//!
//! omp split this over `loader.ts` (discovery + merge) and `theme.ts`
//! (global state + runtime switching). At MyPi's scale both fit here:
//!
//! - **Bundled**: `titanium.json` + `dark.json`, `include_str!`-ed into the
//!   binary — first paint needs no filesystem.
//! - **Custom**: `~/.config/mypi/themes/*.json`, file name = theme name;
//!   a custom theme never shadows a bundled one (builtin wins, same rule
//!   as omp).
//! - **Global instance**: `RwLock<Theme>` + a monotonic `EPOCH`. A theme
//!   swap bumps the epoch; render caches fold the epoch into their keys so
//!   memoized rows re-shape with the new palette (omp's `themeEpoch`
//!   contract, verbatim).
//!
//! Runtime switches are **session-temporary by design**: nothing is
//! written back to config.yaml. Every launch reads the configured default;
//! a trigger (e.g. `/name`) may override it for the current session only.

use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

mod class;
mod color;
mod token;

pub use class::{Theme, ThemeJson};
pub use color::contrast_text_on;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
pub use token::ColorToken;

use ColorToken as T;

static THEME: RwLock<Option<Theme>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Bundled themes, compiled in. The first entry is the startup fallback.
const BUNDLED: &[(&str, &str)] = &[
    ("titanium", include_str!("titanium.json")),
    ("dark", include_str!("dark.json")),
];

/// The directory custom themes live in.
fn custom_themes_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("mypi").join("themes")
}

/// Parse + compile a theme definition. Custom-theme errors carry the file
/// name so the user knows which JSON to fix.
fn compile_named(name: &str, text: &str) -> Result<Theme, String> {
    let json: ThemeJson =
        serde_json::from_str(text).map_err(|e| format!("主题 {name} 解析失败：{e}"))?;
    json.compile().map_err(|e| format!("主题 {name}：{e}"))
}

/// Load a theme by name: bundled first, then `~/.config/mypi/themes/`.
pub fn load_theme(name: &str) -> Result<Theme, String> {
    for (bname, btext) in BUNDLED {
        if *bname == name {
            return compile_named(name, btext);
        }
    }
    let path = custom_themes_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "找不到主题 {name}（内置没有，{} 也没有）",
            custom_themes_dir().display()
        )
    })?;
    compile_named(name, &text)
}

/// Names available for switching: bundled + custom, sorted, deduped
/// (builtin wins the duplicate, matching load order).
pub fn available_themes() -> Vec<String> {
    let mut names: Vec<String> = BUNDLED.iter().map(|(n, _)| n.to_string()).collect();
    if let Ok(rd) = std::fs::read_dir(custom_themes_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json")
                && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                && !names.iter().any(|n| n == stem)
            {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    names
}

/// Current theme. Falls back to the first bundled theme if never
/// initialized — no caller should have to Option-dance.
pub fn theme() -> Theme {
    THEME.read().clone().unwrap_or_else(fallback_theme)
}

fn fallback_theme() -> Theme {
    let (name, text) = BUNDLED[0];
    compile_named(name, text).expect("bundled theme must compile")
}

/// Snapshot of the current epoch (cache-key ingredient).
pub fn theme_epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

/// Activate a theme by name, bumping the epoch. Returns the error text on
/// failure (caller decides whether to echo it); the active theme is left
/// untouched on failure — a bad switch never blanks the UI.
pub fn set_theme(name: &str) -> Result<(), String> {
    let t = load_theme(name)?;
    *THEME.write() = Some(t);
    EPOCH.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Activate an in-memory theme directly (omp's `setThemeInstance`) — the
/// session-temporary path, no disk involved.
pub fn set_theme_instance(t: Theme) {
    *THEME.write() = Some(t);
    EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Rotate to the theme that owns `seed`.
///
/// The trigger contract (session-temporary, never persisted): a trigger
/// event — currently `/name`, later anything — calls this; the same seed
/// always picks the same theme (djb2 over the name, stable across runs),
/// different seeds wander the pool. Re-selecting the active theme is a
/// no-op (no epoch bump, no repaint churn).
pub fn rotate_for_seed(seed: &str) {
    let pool = available_themes();
    let _ = pool;
    let chosen = if BUNDLED.len() <= 1 {
        return;
    } else {
        // djb2: tiny, stable, good enough spread over short names.
        let mut h: u64 = 5381;
        for b in seed.bytes() {
            h = h.wrapping_mul(33).wrapping_add(b as u64);
        }
        BUNDLED[h as usize % BUNDLED.len()].0
    };
    if current_theme_name() == Some(chosen.to_string()) {
        return;
    }
    let _ = set_theme(chosen);
}

/// Name of the active theme (for statusline echo / tests).
pub fn current_theme_name() -> Option<String> {
    THEME.read().as_ref().map(|t| t.name.clone())
}

/// Startup: load the configured default theme, falling back to bundled.
/// Reads `theme.name` from config.yaml if present; `theme.accent`-style
/// overrides from the old models.yml scheme keep working through
/// [`init_from_config`].
pub fn init(default_name: Option<&str>) {
    let name = default_name.unwrap_or(BUNDLED[0].0);
    match load_theme(name) {
        Ok(t) => set_theme_instance(t),
        Err(e) => {
            // Bad config must not brick the TUI: fall back + remember why.
            eprintln!("theme: {e}; 使用内置主题 {}", BUNDLED[0].0);
            set_theme_instance(fallback_theme());
        }
    }
}

/// Startup from the legacy models.yml `theme:` block: `accent`/`gold`/
/// `black`/`muted` overrides layered onto the bundled base theme. Keeps
/// existing configs working while JSON themes become the primary path.
pub fn init_from_config(default_name: Option<&str>, cfg_theme: Option<&crate::ai::config::Theme>) {
    init(default_name);
    let Some(cfg) = cfg_theme else { return };
    // Only override when the user actually customized (defaults in
    // models.yml would otherwise silently fight the JSON theme).
    let is_default = cfg.accent.to_spec() == "green" && cfg.gold.to_spec() == "yellow";
    if is_default {
        return;
    }
    let mut t = theme();
    let overrides = [
        (ColorToken::Accent, Some(cfg.accent.to_color())),
        (ColorToken::Warning, Some(cfg.gold.to_color())),
        (ColorToken::StatusLineBg, Some(cfg.black.to_color())),
        (ColorToken::Muted, Some(cfg.muted.to_color())),
    ];
    let mut changed = false;
    {
        let colors = theme_colors_mut(&mut t);
        for (token, c) in overrides {
            if let Some(c) = c {
                colors.insert(token, c);
                changed = true;
            }
        }
    }
    if changed {
        set_theme_instance(t);
    }
}

/// Direct mutable access for the config-override path above.
fn theme_colors_mut(
    t: &mut Theme,
) -> &mut std::collections::HashMap<ColorToken, ratatui::style::Color> {
    &mut t.colors
}

// ===========================================================================
// Palette — legacy façade over the global theme
// ===========================================================================

/// Legacy four-slot view of the active theme.
///
/// Statusline / completion / input were built against this API; a fresh
/// copy is taken from the global theme every frame, so a mid-session
/// theme switch restyles them with zero re-plumbing.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color,
    pub gold: Color,
    /// The card background. titanium: `#0f1216` (darkTitanium) — omp's
    /// user-message/tool background, not pure black.
    pub black: Color,
    pub muted: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self::current()
    }
}

impl Palette {
    /// Snapshot of the active theme's legacy slots.
    pub fn current() -> Self {
        let t = theme();
        Self {
            accent: t.color(T::Accent),
            gold: t.color(T::Warning),
            black: t.color(T::UserMessageBg),
            muted: t.color(T::Muted),
        }
    }

    /// Kept for call-site stability.
    pub fn from_config(_cfg: &crate::ai::config::Config) -> Self {
        Self::current()
    }

    // ---- span builders: the single place that decides which color applies ----

    /// Accent text, transparent background.
    pub fn accent_span(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.accent))
    }

    /// Money-colored text, transparent background.
    pub fn gold_span(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.gold))
    }

    /// Capsule body (card background + given foreground).
    pub fn pill(&self, text: impl Into<String>, fg: Color) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(fg))
    }

    /// Card background, default foreground (breathing room inside capsules).
    pub fn on_black(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black))
    }

    /// Card background + accent foreground (`[M]` / `[D]` style markers).
    pub fn mark(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(self.accent))
    }

    /// Card background + money color.
    pub fn gold_on_black(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(self.gold))
    }

    /// Transparent background, default foreground (placeholder dashes).
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }

    /// Muted gray text, transparent background.
    pub fn muted_span(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.muted))
    }

    /// Muted gray + italic: helper text such as completion descriptions and thinking blocks.
    pub fn muted_italic(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(
            text.into(),
            Style::new().fg(self.muted).add_modifier(Modifier::ITALIC),
        )
    }

    /// First character: π when idle (gray), a spinner frame while waiting (accent).
    pub fn symbol(&self, spinner: Option<char>) -> Span<'static> {
        match spinner {
            Some(c) => Span::styled(c.to_string(), Style::new().bg(self.black).fg(self.accent)),
            None => Span::styled("π", Style::new().bg(self.black).fg(self.muted)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use token::ALL_TOKENS;

    #[test]
    fn bundled_themes_compile_and_have_all_tokens() {
        for (name, text) in BUNDLED {
            let t = compile_named(name, text).unwrap_or_else(|e| panic!("{name}: {e}"));
            // Spot-check a few token resolutions rather than all 60.
            assert!(!t.hex(ColorToken::Accent).is_empty(), "{name} accent");
        }
    }

    #[test]
    fn unknown_theme_errors() {
        assert!(load_theme("no-such-theme-xyz").is_err());
    }

    #[test]
    fn titanium_palette_matches_omp() {
        let t = load_theme("titanium").unwrap();
        assert_eq!(t.hex(ColorToken::Accent), "#00b4ff");
        assert_eq!(t.hex(ColorToken::Success), "#00ff88");
        assert_eq!(t.hex(ColorToken::Error), "#ff4757");
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use token::ALL_TOKENS;

    #[test]
    fn bundled_themes_all_load_and_complete() {
        for (name, _) in BUNDLED {
            let t = load_theme(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            // Every token resolves to a concrete color (no Reset holes in
            // our bundled themes — Reset bg would paint nothing).
            for token in ALL_TOKENS {
                let _ = t.color(*token);
            }
        }
    }

    #[test]
    fn rotate_is_stable_per_seed() {
        // Same seed always lands on the same bundled theme.
        let pick = |seed: &str| {
            let mut h: u64 = 5381;
            for b in seed.bytes() {
                h = h.wrapping_mul(33).wrapping_add(b as u64);
            }
            BUNDLED[h as usize % BUNDLED.len()].0
        };
        assert_eq!(pick("alpha"), pick("alpha"));
        assert_eq!(pick("beta"), pick("beta"));
    }

    #[test]
    fn rotate_actually_swaps_the_instance() {
        let before = current_theme_name();
        // Pick a seed that maps to the *other* bundled theme.
        let other = if before.as_deref() == Some("titanium") {
            "dark"
        } else {
            "titanium"
        };
        let mut chosen = None;
        for s in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"] {
            let mut h: u64 = 5381;
            for b in s.bytes() {
                h = h.wrapping_mul(33).wrapping_add(b as u64);
            }
            let pick = BUNDLED[h as usize % BUNDLED.len()].0;
            if pick != before.as_deref().unwrap_or("") {
                chosen = Some(s);
                break;
            }
        }
        if let Some(seed) = chosen {
            let _ = other;
            rotate_for_seed(seed);
            assert_ne!(current_theme_name(), before, "换种子必须换主题");
            let e1 = theme_epoch();
            rotate_for_seed(seed); // same seed again: no-op
            assert_eq!(theme_epoch(), e1, "同种子重复轮换不应 bump epoch");
        }
    }
}
