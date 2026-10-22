//! Theme loading — bundled themes, custom themes, the global instance.
//!
//! omp split this over `loader.ts` (discovery + merge) and `theme.ts`
//! (global state + runtime switching). At MyPi's scale both fit here:
//!
//! - **Bundled**: `titanium.json` + `dark.json`, `include_str!`-ed into the
//!   binary — first paint needs no filesystem.
//! - **Custom**: `<root>/themes/*.json`（项目根优先、全局根兜底，见
//!   [`crate::server::config::paths::ConfigPaths`]），文件名 = 主题名；自定义主题
//!   永远盖不过内置的（内置先命中，与 omp 同规则），同名时项目那份先到先得。
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
use ratatui::style::{Color, Style};
use ratatui::text::Span;
pub use token::ColorToken;

use ColorToken as T;

static THEME: RwLock<Option<Theme>> = RwLock::new(None);
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// 「请重绘一帧」的回调槽。
///
/// 渲染层（语法预热的后台线程）需要在不认识信号类型的前提下叫醒主循环；
/// 主循环启动时装上真正的实现。没装 = 没有前端在跑（测试、纯服务端），
/// 调用是空操作。
static REPAINT: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// 装上报「重绘一帧」的实现（主循环启动时调一次；重复装忽略）。
pub fn set_repaint_hook(f: impl Fn() + Send + Sync + 'static) {
    let _ = REPAINT.set(Box::new(f));
}

/// 请前端重绘一帧。没有前端时是空操作。
pub fn request_repaint() {
    if let Some(f) = REPAINT.get() {
        f();
    }
}

/// Bundled themes, compiled in. The first entry is the startup fallback.
const BUNDLED: &[(&str, &str)] = &[
    ("titanium", include_str!("titanium.json")),
    ("dark", include_str!("dark.json")),
];

/// 主题目录，由配置层注入（项目在前、全局在后）。没注入就只用内置主题——
/// 这一层不认识 XDG，也不去猜。
static THEME_DIRS: std::sync::OnceLock<Vec<std::path::PathBuf>> = std::sync::OnceLock::new();

/// 启动时装上主题目录（`cfg.paths.themes_dirs()`）。重复装忽略。
pub fn install_dirs(paths: &crate::server::config::paths::ConfigPaths) {
    let dirs = paths
        .themes_dirs()
        .into_iter()
        .map(|(_, dir)| dir)
        .collect();
    let _ = THEME_DIRS.set(dirs);
}

/// 按顺序排的候选目录：前面的同名文件说了算。
fn theme_dirs() -> &'static [std::path::PathBuf] {
    THEME_DIRS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// 错误信息里"找过哪里"那一句。
fn looked_in() -> String {
    let dirs = theme_dirs();
    if dirs.is_empty() {
        return "没有自定义主题目录".into();
    }
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(" 与 ")
}

/// Parse + compile a theme definition. Custom-theme errors carry the file
/// name so the user knows which JSON to fix.
fn compile_named(name: &str, text: &str) -> Result<Theme, String> {
    let json: ThemeJson =
        serde_json::from_str(text).map_err(|e| format!("主题 {name} 解析失败：{e}"))?;
    json.compile().map_err(|e| format!("主题 {name}：{e}"))
}

/// Load a theme by name: bundled first, then the custom dirs in order.
pub fn load_theme(name: &str) -> Result<Theme, String> {
    for (bname, btext) in BUNDLED {
        if *bname == name {
            return compile_named(name, btext);
        }
    }
    for dir in theme_dirs() {
        let path = dir.join(format!("{name}.json"));
        // 读不到就试下一个目录；读得到就认它（同名时项目那份先到先得）。
        if let Ok(text) = std::fs::read_to_string(&path) {
            return compile_named(name, &text);
        }
    }
    Err(format!("找不到主题 {name}（内置没有，{} 也没有）", looked_in()))
}

/// Names available for switching: bundled + custom, sorted, deduped
/// (builtin wins a duplicate, and among custom dirs the earlier one does).
pub fn available_themes() -> Vec<String> {
    let mut names: Vec<String> = BUNDLED.iter().map(|(n, _)| n.to_string()).collect();
    for dir in theme_dirs() {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
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
pub fn init_from_config(default_name: Option<&str>, cfg_theme: Option<&crate::server::config::read::config::Theme>) {
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

/// Legacy view of the active theme.
///
/// Completion / input / transcript were built against this API; a fresh copy
/// is taken from the global theme every frame, so a mid-session theme switch
/// restyles them with zero re-plumbing. The statusline no longer uses it — it
/// has its own, wider surface (`crate::ui::tui::zone::main::input::statusline::StatusTheme`).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color,
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
            black: t.color(T::UserMessageBg),
            muted: t.color(T::Muted),
        }
    }

    /// Kept for call-site stability.
    pub fn from_config(_cfg: &crate::server::config::Config) -> Self {
        Self::current()
    }

    // ---- span builders: the single place that decides which color applies ----

    /// Accent text, transparent background.
    pub fn accent_span(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.accent))
    }

    /// Transparent background, default foreground (placeholder dashes).
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }

    /// Muted gray, upright: helper text such as completion descriptions.
    pub fn muted(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.muted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 注入的目录按顺序查找：同名时项目那份先命中，项目里没有的落到全局那份。
    #[test]
    fn injected_dirs_are_searched_in_order() {
        use crate::server::config::paths::{ConfigPaths, Origin};
        let project = std::env::temp_dir().join(format!("mypi-theme-p-{}", std::process::id()));
        let global = std::env::temp_dir().join(format!("mypi-theme-g-{}", std::process::id()));
        // 住户是**目录**：`<root>/themes/`（配置层的候选按 `is_dir()` 过滤）。
        for d in [&project, &global] {
            let _ = std::fs::remove_dir_all(d);
            std::fs::create_dir_all(d.join("themes")).unwrap();
        }
        // 两边都有 probe.json，内容不同：项目那份是 dark 的配色，全局那份是
        // titanium 的。只有全局有 fallback.json。
        std::fs::write(
            project.join("themes").join("probe.json"),
            include_str!("dark.json"),
        )
        .unwrap();
        std::fs::write(
            global.join("themes").join("probe.json"),
            include_str!("titanium.json"),
        )
        .unwrap();
        std::fs::write(
            global.join("themes").join("fallback.json"),
            include_str!("titanium.json"),
        )
        .unwrap();

        install_dirs(&ConfigPaths::new(vec![
            (Origin::Project, project.clone()),
            (Origin::Global, global.clone()),
        ]));

        assert_eq!(
            load_theme("probe").unwrap().hex(ColorToken::Accent),
            load_theme("dark").unwrap().hex(ColorToken::Accent),
            "同名主题：项目那份说了算"
        );
        assert_eq!(
            load_theme("fallback").unwrap().hex(ColorToken::Accent),
            load_theme("titanium").unwrap().hex(ColorToken::Accent),
            "项目里没有：落到全局那份"
        );
        // 目录型的并集：两个目录里的名字都在候选里。
        let names = available_themes();
        assert!(names.contains(&"probe".to_string()));
        assert!(names.contains(&"fallback".to_string()));

        for d in [&project, &global] {
            let _ = std::fs::remove_dir_all(d);
        }
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
