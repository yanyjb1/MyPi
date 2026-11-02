//! The `Theme` class — omp `theme/theme-class.ts` ported to ratatui.
//!
//! A theme is built once from JSON: every token's value is resolved (vars
//! dereferenced) and precompiled into a `ratatui::style::Color` plus a hex
//! string. Runtime lookups are pure table reads — zero color math per
//! frame, same as omp's precompiled-ANSI design but targeting ratatui
//! styles instead of raw escape sequences (crossterm does the emitting).
//!
//! Span helpers mirror omp's `fg`/`bg`/`fgOnBg`: they *only* set the
//! aspect they name (foreground helpers never touch background), so
//! layered styling composes the way omp cards do.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use super::color::parse_hex;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::token::{ALL_TOKENS, ColorToken};

/// One theme: 60 resolved tokens + the name it came from.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub(crate) colors: HashMap<ColorToken, Color>,
    hexes: HashMap<ColorToken, String>,
}

/// A color value as it appears in theme JSON before resolution:
/// `""` (terminal default), `"#rrggbb"`, `"varName"`, or 0-255 index.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ColorValue {
    Index(u8),
    Str(String),
}

impl ColorValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            ColorValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// Raw theme JSON: vars + colors (+ symbols reserved for a later slice).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThemeJson {
    pub name: String,
    #[serde(default)]
    pub vars: std::collections::HashMap<String, ColorValue>,
    #[serde(default)]
    pub colors: std::collections::HashMap<String, ColorValue>,
    #[serde(default)]
    pub symbols: SymbolsSection,
}

/// Symbols section — parsed and validated but the preset table itself is a
/// later slice; for now the field exists so omp-format themes load as-is.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SymbolsSection {
    #[serde(default)]
    pub preset: Option<String>,
}

impl ThemeJson {
    /// Resolve vars and precompile every token.
    ///
    /// Missing tokens are an error listing what's absent (omp's loader
    /// behavior — the message tells the theme author exactly what to add).
    /// Unknown extra keys are ignored (forward compatibility).
    pub fn compile(self) -> Result<Theme, String> {
        self.compile_checked(true)
    }

    /// `require_all = false` skips the full-token audit (unit tests build
    /// tiny partial themes; real loads always audit).
    pub fn compile_checked(self, require_all: bool) -> Result<Theme, String> {
        let mut colors = HashMap::new();
        let mut hexes = HashMap::new();
        let mut missing = Vec::new();
        for token in ALL_TOKENS {
            let key = token.json_key();
            let Some(value) = self.colors.get(key) else {
                if require_all {
                    missing.push(key);
                }
                continue;
            };
            match resolve_value(value, &self.vars, &mut 0) {
                Some(c) => {
                    let hex = hex_of(c);
                    colors.insert(*token, c);
                    hexes.insert(*token, hex);
                }
                None => {
                    return Err(format!(
                        "无法解析颜色 {key} = {:?}",
                        value.as_str().unwrap_or("<index>")
                    ));
                }
            }
        }
        if !missing.is_empty() {
            return Err(format!("主题缺少颜色 token：{}", missing.join(", ")));
        }
        Ok(Theme {
            name: self.name,
            colors,
            hexes,
        })
    }
}

/// Dereference a value through `vars` (cycle-safe, depth-limited).
fn resolve_value(
    value: &ColorValue,
    vars: &std::collections::HashMap<String, ColorValue>,
    depth: &mut u8,
) -> Option<Color> {
    if *depth > 8 {
        return None; // cycle or absurd chain
    }
    match value {
        ColorValue::Index(i) => Some(Color::Indexed(*i)),
        ColorValue::Str(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Some(Color::Reset); // terminal default
            }
            if let Some(rgb) = parse_hex(t) {
                return Some(Color::Rgb(rgb.0, rgb.1, rgb.2));
            }
            // Named ANSI colors (a convenience omp doesn't have; our old
            // models.yml themes used them, keep them loadable).
            if let Some(c) = super::named_color(t) {
                return Some(c);
            }
            // var reference
            let next = vars.get(t)?;
            *depth += 1;
            resolve_value(next, vars, depth)
        }
    }
}

fn hex_of(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => String::new(),
    }
}

impl Theme {
    /// A token's resolved color. Panics on an unknown token — a coding
    /// error, not data: every token is guaranteed present by `compile`.
    pub fn color(&self, token: ColorToken) -> Color {
        self.colors[&token]
    }

    /// A token's hex form (empty for non-RGB). Accent math and tests.
    pub fn hex(&self, token: ColorToken) -> &str {
        &self.hexes[&token]
    }

    /// Token as a ratatui foreground style.
    pub fn fg_style(&self, token: ColorToken) -> Style {
        Style::new().fg(self.color(token))
    }

    /// Token as a ratatui background style.
    pub fn bg_style(&self, token: ColorToken) -> Style {
        Style::new().bg(self.color(token))
    }

    // ---- span builders: the omp fg()/bg() ergonomics ----

    /// Foreground-colored span (background untouched).
    pub fn fg(&self, token: ColorToken, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), self.fg_style(token))
    }

    /// Background-filled span (foreground untouched).
    pub fn bg(&self, token: ColorToken, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), self.bg_style(token))
    }

    /// Convenience: token foreground + modifier set.
    pub fn fg_mod(
        &self,
        token: ColorToken,
        text: impl Into<String>,
        m: ratatui::style::Modifier,
    ) -> Span<'static> {
        Span::styled(text.into(), self.fg_style(token).add_modifier(m))
    }
}
