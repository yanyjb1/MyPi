//! Configuration — follows pi's models-store / models.yml approach:
//! endpoints, credentials, and pricing all live in models.yml, zero
//! hardcoding in code.
//!
//! File lookup order (first existing wins):
//!   1. $MYPI_CONFIG (explicit override; used by tests)
//!   2. $XDG_CONFIG_HOME/mypi/config.yaml (canonical; default ~/.config/mypi/config.yaml)
//!
//! The repo ships `models_example.yml` as a template — copy it to the
//! canonical location and fill in your keys. No CWD fallback: the repo
//! never holds credentials.
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
    pub theme: Theme,
}

/// Connection details for one provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Provider {
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// Wire dialect. All providers speak an OpenAI-compatible protocol;
    /// this field only selects **vendor-specific deltas** on top of it.
    /// Values: `openai-compatible` (default) | `deepseek`.
    /// The user opts into a vendor's quirks explicitly — the code ships
    /// the capability, the config decides whether it is used.
    #[serde(default, rename = "api")]
    pub api: Option<String>,
    /// Models this provider serves — **nested inside the provider**, never
    /// a top-level flat list. The model id is provider-internal: the same
    /// gateway can expose `gpt-x`, DeepSeek's API exposes
    /// `deepseek-chat`; ids are written per provider by the user.
    #[serde(default, rename = "models")]
    pub models: Vec<ModelEntry>,
}

impl Provider {
    /// Resolved dialect (config value or the default).
    pub fn dialect(&self) -> Dialect {
        match self.api.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) if v.eq_ignore_ascii_case("deepseek") => Dialect::DeepSeek,
            _ => Dialect::OpenAiCompatible,
        }
    }
}

/// A model plus the provider it lives under (the config nests models
/// inside providers; callers always need both).
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub entry: ModelEntry,
}

/// Vendor-specific deltas over the OpenAI-compatible wire protocol.
/// Not a protocol family — all of these still speak openai-completions;
/// the variant only enables vendor quirks (e.g. DeepSeek's separate
/// reasoning stream that gates the visible content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Plain OpenAI protocol; reasoning arrives as `reasoning_content`-style
    /// fields when the model offers it.
    OpenAiCompatible,
    /// DeepSeek: `reasoning_content` streams **before** `content`, and
    /// content must not be rendered until the reasoning stream closes.
    DeepSeek,
}

/// One callable model (a `- id: ...` entry in models.yml).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelEntry {
    /// Model id sent to the server ("vendor:name" form).
    pub id: String,
    /// Statusline display name (e.g. GLM5.3F(MS)).
    #[serde(default)]
    pub name: String,
    /// Context window (tokens); denominator of the statusline gauge.
    #[serde(default, rename = "contextWindow")]
    pub context_window: u64,
    /// User-capped output per reply — the actual `max_tokens` sent in
    /// every request. Optional because providers **reject requests that
    /// exceed the model's real limit**: a 128k-capable model must not be
    /// told 64000 unless you know your tier serves it. Omit for the
    /// conservative default (4096).
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
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "no config at {} — create it from models_example.yml (`mkdir -p ~/.config/mypi && cp models_example.yml ~/.config/mypi/config.yaml`), then fill in your providers and keys",
                path.display()
            )
        })?;
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
        // XDG only — no CWD fallback. The repo never holds credentials;
        // `models_example.yml` documents the format and users copy it to
        // the canonical location.
        Ok(xdg.join("mypi").join("config.yaml"))
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

    /// Fail fast: model ids are **provider-scoped** (the same id may
    /// appear under two providers), so uniqueness is checked per provider.
    /// The globally-addressed form is `<provider>:<id>` — enforced where
    /// models are looked up, not here.
    fn validate(&self) -> anyhow::Result<()> {
        for (pname, p) in &self.providers {
            let mut seen = std::collections::HashSet::new();
            for m in &p.models {
                if !seen.insert(&m.id) {
                    return Err(anyhow!("provider {pname}: duplicate model id `{}`", m.id));
                }
            }
        }
        Ok(())
    }

    /// Every declared model with its owning provider name, in config
    /// order (providers are a map, so sort names for a stable listing).
    pub fn models(&self) -> impl Iterator<Item = (&str, &ModelEntry)> {
        let mut names: Vec<&String> = self.providers.keys().collect();
        names.sort();
        names
            .into_iter()
            .flat_map(move |n| self.providers[n.as_str()].models.iter().map(move |m| (n.as_str(), m)))
    }

    /// Fetch the default model entry. The `default:` key is **mandatory**
    /// — declared by the user in the config, never inferred (no
    /// first-model fallback, no implicit default).
    pub fn default_model(&self) -> anyhow::Result<ResolvedModel> {
        let id = self
            .default
            .as_deref()
            .ok_or_else(|| anyhow!("config has no `default:` model — add e.g. `default: local:gpt-5.6-luna` to the config"))?;
        self.model_by_id(id)
    }

    /// Find a model by its globally-addressed id `<provider>:<id>`
    /// (the form used by `default`, /model, /switch and completion).
    pub fn model_by_id(&self, id: &str) -> anyhow::Result<ResolvedModel> {
        let (pname, mid) = id.split_once(':').ok_or_else(|| {
            anyhow!("model id must be `<provider>:<id>`, got `{id}`")
        })?;
        let p = self.providers.get(pname).ok_or_else(|| anyhow!("unknown provider: {pname}"))?;
        let m = p.models.iter().find(|m| m.id == mid)
            .ok_or_else(|| anyhow!("provider {pname} has no model `{mid}`"))?;
        Ok(ResolvedModel { provider_name: pname.to_string(), entry: m.clone() })
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
    fn parses_nested_provider_models() {
        let cfg: Config = serde_yaml::from_str(
            r#"
providers:
  local:
    base_url: http://localhost:9999/v1
    api_key: ""
    models:
      - id: vendor-a/model-x
        name: ModelX
        contextWindow: 1000000
        maxOutputTokens: 128000
        currency: Cny
        cost:
          input: 1
          output: 4
          cacheRead: 0.02
          cacheWrite: 0.0
      - id: vendor-b/model-y
        contextWindow: 1000000
        cost: { input: 0.8, output: 2.7, cacheRead: 0.1, cacheWrite: 1.25 }
default: local:vendor-a/model-x
"#,
        )
        .unwrap();
        // Addressing is <provider>:<id>; the model entry itself keeps the bare id.
        let m = cfg.default_model().unwrap();
        assert_eq!(m.provider_name, "local");
        assert_eq!(m.entry.id, "vendor-a/model-x");
        assert_eq!(Config::display_name(&m.entry), "ModelX");
        assert_eq!(m.entry.cost.input, 1.0);
        assert_eq!(m.entry.cost.cache_write, 0.0);
        assert_eq!(m.entry.currency, Currency::Cny);
        // Second model has no name/currency: display falls back to id, currency defaults to Cny
        let q = cfg.model_by_id("local:vendor-b/model-y").unwrap();
        assert_eq!(Config::display_name(&q.entry), "vendor-b/model-y");
        assert_eq!(q.entry.currency, Currency::Cny);
        // Bare id without provider prefix is rejected with guidance.
        assert!(cfg.model_by_id("vendor-b/model-y").is_err());
        // Cross-provider id does not leak into another provider's list.
        assert!(cfg.model_by_id("nope:vendor-b/model-y").is_err());
    }

    /// `default:` is mandatory — no first-model fallback.
    #[test]
    fn default_is_mandatory() {
        let cfg: Config = serde_yaml::from_str(
            r#"
providers:
  local: { base_url: "http://x/v1", models: [{ id: m1 }] }
"#,
        )
        .unwrap();
        let err = cfg.default_model().unwrap_err().to_string();
        assert!(err.contains("no `default:` model"), "{err}");
    }

    /// A missing config file is a hard error pointing at the template.
    #[test]
    fn load_errors_when_config_missing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-missing-{}", std::process::id()));
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            std::env::remove_var("MYPI_CONFIG");
        }
        let err = Config::load().unwrap_err().to_string();
        assert!(err.contains("create it from models_example.yml"), "{err}");
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
    fn locate_prefers_mypi_config_env_then_xdg() {
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
        std::env::set_current_dir(std::env::temp_dir()).unwrap(); // XDG only: CWD is irrelevant now
        let p = Config::locate().unwrap();
        assert_eq!(p, mypi.join("config.yaml"));

        // 3) canonical path missing -> still returns the canonical path
        //    (no CWD fallback; the error message must point at the
        //    expected location)
        std::fs::remove_file(mypi.join("config.yaml")).unwrap();
        let p = Config::locate().unwrap();
        assert_eq!(p, mypi.join("config.yaml"));
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
        let yaml = "providers:\n  local:\n    base_url: http://x/v1\n    models:\n      - id: a\n      - id: b\ndefault: local:a\n";
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.save_default("local:b").unwrap();
        let text = std::fs::read_to_string(dir.join("mypi").join("config.yaml")).unwrap();
        let back: Config = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.default.as_deref(), Some("local:b"));
        assert_eq!(back.models().count(), 2, "other config sections preserved verbatim");
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
