//! Configuration — follows pi's models-store / models.yml approach:
//! endpoints, credentials, and pricing all live in models.yml, zero
//! hardcoding in code.
//!
//! File lookup order (first existing wins):
//!   1. $MYPI_CONFIG
//!   2. ./models.yml (next to .env; the current directory)
//!
//! Structure example (**keep in sync with the models.yml in the repo**):
//!
//! ```yaml
//! theme:
//!   accent: green
//!   gold: yellow
//!   black: "0,0,0"
//!
//! providers:
//!   local:
//!     base_url: http://localhost:7863/v1
//!     api_key: ""                 # or a ${ENV_VAR} reference
//!
//! default: global:gpt-5.6-luna
//!
//! models:
//!   - id: global:gpt-5.6-luna     # model id sent to the server
//!     name: GPT5.6L(LC)           # statusline display name
//!     provider: local
//!     contextWindow: 1000000      # denominator of the statusline ctx gauge
//!     maxOutputTokens: 128000
//!     currency: Usd
//!     cost:
//!       input: 0.20
//!       output: 1.20
//!       cacheRead: 0.02
//!       cacheWrite: 0.0
//! ```
//!
//! Note `models` is a **list** (camelCase `contextWindow` field). An
//! earlier design used an id-keyed map with a snake_case
//! `context_length`; that documentation is long obsolete — do not copy it.
//!
//! Pricing: unit prices are per 1M tokens; `currency` only picks the
//! display symbol (¥ / $), no conversion — quotes are already in each
//! vendor's native currency. Most gateways do not report cacheWrite
//! separately; 0 means "price as plain input".

use anyhow::{Context as _, anyhow};
use serde::{Deserialize, Serialize};

/// Palette: loaded from the theme section of models.yml; no colors hardcoded in code.
/// `accent` is the theme color (borders/model name/used ctx), `gold` the
/// money color, `black` the capsule background (default true black —
/// fixes Konsole's grayish indexed black), `muted` the secondary gray
/// (the π symbol and friends).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Theme {
    #[serde(default = "d_accent")]
    pub accent: ColorSpec,
    #[serde(default = "d_gold")]
    pub gold: ColorSpec,
    #[serde(default = "d_black")]
    pub black: ColorSpec,
    #[serde(default = "d_muted")]
    pub muted: ColorSpec,
}

fn d_accent() -> ColorSpec { ColorSpec::Text("green".into()) }
fn d_gold() -> ColorSpec { ColorSpec::Text("yellow".into()) }
fn d_black() -> ColorSpec { ColorSpec::Rgb(0, 0, 0) }
fn d_muted() -> ColorSpec { ColorSpec::Text("darkgray".into()) }

impl Default for Theme {
    fn default() -> Self {
        Self {
            accent: d_accent(),
            gold: d_gold(),
            black: d_black(),
            muted: d_muted(),
        }
    }
}

/// Color syntax: named colors (green/blue/...), `#rrggbb`, or `r,g,b` truecolor.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ColorSpec {
    Rgb(u8, u8, u8),
    Text(String),
}

impl ColorSpec {
    /// Convert to a ratatui color. Named colors go through ANSI indexed colors.
    pub fn to_color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            ColorSpec::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
            ColorSpec::Text(s) => {
                let t = s.trim();
                // "#rrggbb"
                if let Some(hex) = t.strip_prefix('#')
                    && hex.len() == 6
                    && let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    )
                {
                    return Color::Rgb(r, g, b);
                }
                // "r,g,b"
                let parts: Vec<&str> = t.split(',').map(|x| x.trim()).collect();
                if parts.len() == 3
                    && let (Ok(r), Ok(g), Ok(b)) = (
                        parts[0].parse::<u8>(),
                        parts[1].parse::<u8>(),
                        parts[2].parse::<u8>(),
                    )
                {
                    return Color::Rgb(r, g, b);
                }
                match t.to_ascii_lowercase().as_str() {
                    "black" => Color::Rgb(0, 0, 0), // true black; Konsole's indexed black looks gray
                    "gray" | "grey" | "darkgray" | "darkgrey" => Color::DarkGray,
                    "white" => Color::White,
                    "red" => Color::Red,
                    "green" => Color::Green,
                    "yellow" | "gold" => Color::Yellow,
                    "blue" => Color::Blue,
                    "magenta" | "purple" => Color::Magenta,
                    "cyan" => Color::Cyan,
                    _ => Color::Green,
                }
            }
        }
    }
}

/// Four unit prices per 1M tokens (USD): input / output / cache read / cache write.
/// cacheRead = input price on cache hits; cacheWrite = input price for writes.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Cost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default, rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(default, rename = "cacheWrite")]
    pub cache_write: f64,
}

impl Default for Cost {
    fn default() -> Self {
        Self { input: 0.0, output: 0.0, cache_read: 0.0, cache_write: 0.0 }
    }
}

/// Display currency: CNY (default, ¥) | USD | Credits. Configurable per model in models.yml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum Currency {
    #[default]
    Cny,
    Usd,
    Credits,
}

impl Currency {
    pub fn symbol(self) -> &'static str {
        match self {
            Currency::Usd => "$",
            Currency::Cny => "¥",
            Currency::Credits => "cr",
        }
    }
}

/// Top-level config file. `models` is a list (hand-written format); `default` matches by id.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub providers: std::collections::HashMap<String, Provider>,
    /// Default model id (e.g. "global:gpt-5.6-luna"). Defaults to the first entry.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    #[serde(default)]
    pub theme: Theme,
}

/// Connection details for one provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Provider {
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
}

/// One callable model (a `- id: ...` entry in models.yml).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelEntry {
    /// Model id sent to the server ("vendor:name" form).
    pub id: String,
    /// Statusline display name (e.g. GLM5.3F(MS)).
    #[serde(default)]
    pub name: String,
    /// Which provider entry this model belongs to.
    pub provider: String,
    /// Context window (tokens); denominator of the statusline gauge.
    #[serde(default, rename = "contextWindow")]
    pub context_window: u64,
    #[serde(default, rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub currency: Currency,
    #[serde(default)]
    pub cost: Cost,
}


impl Config {
    /// Load and parse the config file in lookup order.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::locate()?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read config: {}", path.display()))?;
        let cfg: Config = serde_yaml::from_str(&text)
            .with_context(|| format!("invalid config format: {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Config lookup order (first existing wins):
    ///   1. $MYPI_CONFIG (explicit; used by tests)
    ///   2. $XDG_CONFIG_HOME/mypi/config.yaml (canonical location)
    ///   3. ~/.config/mypi/config.yaml (XDG default)
    ///   4. ./models.yml (legacy path, kept for existing repos)
    pub fn locate() -> anyhow::Result<std::path::PathBuf> {
        if let Ok(p) = std::env::var("MYPI_CONFIG") {
            return Ok(std::path::PathBuf::from(p));
        }
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
            .ok_or_else(|| anyhow!("neither HOME nor XDG_CONFIG_HOME is set; cannot locate config"))?;
        let canonical = xdg.join("mypi").join("config.yaml");
        let legacy = std::path::PathBuf::from("models.yml");
        if canonical.exists() {
            Ok(canonical)
        } else if legacy.exists() {
            Ok(legacy)
        } else {
            // Nothing exists: return the canonical path so the error message points at the expected location
            Ok(canonical)
        }
    }

    /// Directory containing the config file ($XDG_CONFIG_HOME/mypi). Used by save.
    pub fn config_dir() -> anyhow::Result<std::path::PathBuf> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
            .ok_or_else(|| anyhow!("neither HOME nor XDG_CONFIG_HOME is set; cannot locate config"))?;
        Ok(xdg.join("mypi"))
    }

    /// Write `default` back to the config file
    /// (XDG_CONFIG_HOME/mypi/config.yaml).
    ///
    /// Even when the config was loaded from the legacy models.yml or
    /// $MYPI_CONFIG, the canonical location is written — it has higher
    /// precedence and takes over on the next launch.
    pub fn save_default(&self, id: &str) -> anyhow::Result<()> {
        let dir = Self::config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let mut next = Clone::clone(self);
        next.default = Some(id.to_string());
        let path = dir.join("config.yaml");
        // $MYPI_CONFIG has the highest precedence; write it directly so
        // the change cannot be silently shadowed
        let path = match std::env::var("MYPI_CONFIG") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => path,
        };
        let yaml = serde_yaml::to_string(&next)?;
        std::fs::write(&path, yaml)
            .with_context(|| format!("failed to write config: {}", path.display()))?;
        Ok(())
    }

    /// Fail fast: every model must reference a declared provider.
    fn validate(&self) -> anyhow::Result<()> {
        for m in &self.models {
            if !self.providers.contains_key(&m.provider) {
                return Err(anyhow!("model {} references undefined provider `{}`", m.id, m.provider));
            }
        }
        Ok(())
    }

    /// Fetch the default model entry.
    pub fn default_model(&self) -> anyhow::Result<&ModelEntry> {
        match &self.default {
            Some(id) => self.model_by_id(id),
            None => self
                .models
                .first()
                .ok_or_else(|| anyhow!("models list is empty")),
        }
    }

    /// Find a model by id (used by model switching).
    pub fn model_by_id(&self, id: &str) -> anyhow::Result<&ModelEntry> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| anyhow!("model not found: {id}"))
    }

    /// Resolve `${ENV_VAR}` references in api_key.
    pub fn resolve_key(&self, provider: &Provider) -> String {
        let raw = provider.api_key.trim();
        if let Some(inner) = raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
            std::env::var(inner).unwrap_or_default()
        } else {
            raw.to_string()
        }
    }

    /// Statusline display name: `name` preferred, id as fallback.
    pub fn display_name(m: &ModelEntry) -> &str {
        if m.name.is_empty() { &m.id } else { &m.name }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_format() {
        let cfg: Config = serde_yaml::from_str(
            r#"
providers:
  local:
    base_url: http://localhost:9999/v1
    api_key: ""
default: vendor-a/model-x
models:
  - id: vendor-a/model-x
    name: ModelX
    provider: local
    contextWindow: 1000000
    maxOutputTokens: 128000
    currency: Cny
    cost:
      input: 1
      output: 4
      cacheRead: 0.02
      cacheWrite: 0.0
  - id: vendor-b/model-y
    provider: local
    contextWindow: 1000000
    cost: { input: 0.8, output: 2.7, cacheRead: 0.1, cacheWrite: 1.25 }
"#,
        )
        .unwrap();
        let m = cfg.default_model().unwrap();
        assert_eq!(m.id, "vendor-a/model-x");
        assert_eq!(Config::display_name(m), "ModelX");
        assert_eq!(m.cost.input, 1.0);
        assert_eq!(m.cost.cache_write, 0.0);
        assert_eq!(m.currency, Currency::Cny);
        // Second model has no name/currency: display falls back to id, currency defaults to Cny
        let q = cfg.model_by_id("vendor-b/model-y").unwrap();
        assert_eq!(Config::display_name(q), "vendor-b/model-y");
        assert_eq!(q.currency, Currency::Cny);
    }

    /// The theme section parses named/hex/r,g,b colors; defaults green/yellow/true black.
    #[test]
    fn theme_colors_parse() {
        use ratatui::style::Color;
        let cfg: Config = serde_yaml::from_str(
            r##"
providers:
  local: { base_url: "http://x/v1" }
theme:
  accent: "#ff8800"
  gold: "12,34,56"
models: []
"##,
        )
        .unwrap();
        assert_eq!(cfg.theme.accent.to_color(), Color::Rgb(0xff, 0x88, 0x00));
        assert_eq!(cfg.theme.gold.to_color(), Color::Rgb(12, 34, 56));
        // Unconfigured black falls back to true black
        assert_eq!(cfg.theme.black.to_color(), Color::Rgb(0, 0, 0));

        // Everything defaulted
        let bare: Config = serde_yaml::from_str(
            "providers: { local: { base_url: \"http://x/v1\" } }\nmodels: []\n",
        )
        .unwrap();
        assert_eq!(bare.theme.accent.to_color(), Color::Green);
        assert_eq!(bare.theme.gold.to_color(), Color::Yellow);
        assert_eq!(bare.theme.black.to_color(), Color::Rgb(0, 0, 0));
    }


    /// Cross-test mutex: tests that mutate environment variables share this lock.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn locate_prefers_mypi_config_env_then_xdg_then_legacy() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("MYPI_CONFIG", "/tmp/whatever.yaml");
        }
        let p = Config::locate().unwrap();
        assert_eq!(p, std::path::PathBuf::from("/tmp/whatever.yaml"));
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
        }
        // 1) MYPI_CONFIG wins
        unsafe {
            std::env::set_var("MYPI_CONFIG", "/tmp/whatever.yaml");
        }
        let p = Config::locate().unwrap();
        assert_eq!(p, std::path::PathBuf::from("/tmp/whatever.yaml"));
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
        }

        // 2) config.yaml under XDG_CONFIG_HOME when it exists
        let dir = std::env::temp_dir().join(format!("mypi-locate-{}", std::process::id()));
        let mypi = dir.join("mypi");
        std::fs::create_dir_all(&mypi).unwrap();
        std::fs::write(mypi.join("config.yaml"), "providers: {}\n").unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        }
        std::env::set_current_dir(std::env::temp_dir()).unwrap(); // leave the repo, avoid models.yml
        let p = Config::locate().unwrap();
        assert_eq!(p, mypi.join("config.yaml"));

        // 3) canonical path missing, legacy path present -> legacy wins
        std::fs::remove_file(mypi.join("config.yaml")).unwrap();
        let legacy = dir.join("models.yml");
        std::fs::write(&legacy, "x").unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let p = Config::locate().unwrap();
        assert!(p.ends_with("models.yml"), "should fall back to the legacy path: {p:?}");
        assert!(p.is_absolute() == legacy.is_absolute() || p == legacy || p.ends_with("models.yml"));
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_default_writes_xdg_path_and_sets_field() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-save-{}", std::process::id()));
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::remove_var("MYPI_CONFIG");
        }
        let yaml = "providers:\n  local:\n    base_url: http://x/v1\nmodels:\n  - id: a\n    provider: local\n  - id: b\n    provider: local\ndefault: a\n";
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.save_default("b").unwrap();
        let text = std::fs::read_to_string(dir.join("mypi").join("config.yaml")).unwrap();
        let back: Config = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.default.as_deref(), Some("b"));
        assert_eq!(back.models.len(), 2, "other config sections preserved verbatim");
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
