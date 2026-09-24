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
    /// JSON theme name (bundled `titanium`/`dark` or a file under
    /// ~/.config/mypi/themes). When set, the JSON theme wins; the
    /// accent/gold/black/muted fields below then only matter when they
    /// deviate from their defaults (legacy override path).
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default = "d_accent")]
    pub accent: ColorSpec,
    #[serde(default = "d_gold")]
    pub gold: ColorSpec,
    #[serde(default = "d_black")]
    pub black: ColorSpec,
    #[serde(default = "d_muted")]
    pub muted: ColorSpec,
}

fn d_accent() -> ColorSpec {
    ColorSpec::Text("green".into())
}
fn d_gold() -> ColorSpec {
    ColorSpec::Text("yellow".into())
}
fn d_black() -> ColorSpec {
    ColorSpec::Rgb(0, 0, 0)
}
fn d_muted() -> ColorSpec {
    ColorSpec::Text("darkgray".into())
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: None,
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
    /// Original textual spec (for the theme override detector).
    pub fn to_spec(&self) -> String {
        match self {
            ColorSpec::Rgb(r, g, b) => format!("{r},{g},{b}"),
            ColorSpec::Text(s) => s.clone(),
        }
    }

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
        Self {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        }
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

/// **models.yml** — the user-maintained file of providers and models
/// (credentials live here!). The program **never writes** this file:
/// an editor bug must not be able to corrupt the user's keys.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsConfig {
    pub providers: std::collections::HashMap<String, Provider>,
}

/// **config.yaml** — program-managed miscellany (default model, theme).
/// Safe to rewrite: the program owns this file.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AppConfig {
    /// Default model id, addressed as `<provider>:<model id>`.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub theme: Theme,
}

/// The merged view both files feed into (what the rest of the program sees).
#[derive(Debug, Clone)]
pub struct Config {
    pub models: ModelsConfig,
    pub app: AppConfig,
}

/// Connection details for one provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Provider {
    #[serde(alias = "baseUrl")]
    pub base_url: String,
    #[serde(default, alias = "apiKey")]
    pub api_key: String,
    /// Wire dialect — **required** (copy it from the template). Values:
    /// `openai-completions` (OpenAI-compatible) | `deepseek`.
    #[serde(rename = "api")]
    pub api: String,
    /// Models this provider serves — **nested inside the provider**, never
    /// a top-level flat list. The model id is provider-internal: the same
    /// gateway can expose `gpt-x`, DeepSeek's API exposes
    /// `deepseek-chat`; ids are written per provider by the user.
    #[serde(default, rename = "models")]
    pub models: Vec<ModelEntry>,
}

impl Provider {
    /// Resolved dialect (the required `api` value).
    pub fn dialect(&self) -> Dialect {
        match self.api.trim() {
            v if v.eq_ignore_ascii_case("deepseek") => Dialect::DeepSeek,
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

impl ModelEntry {
    /// Human-facing name: the user-chosen `name` field, falling back to
    /// the wire id. Display only — the id is what the server sees.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

impl Config {
    /// XDG base for config files ($XDG_CONFIG_HOME, default ~/.config).
    fn xdg_config_base() -> anyhow::Result<std::path::PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
            .ok_or_else(|| anyhow!("neither HOME nor XDG_CONFIG_HOME is set; cannot locate config"))
    }

    /// models.yml path: $MYPI_MODELS overrides, else $XDG_CONFIG_HOME/mypi/models.yml.
    pub fn models_path() -> anyhow::Result<std::path::PathBuf> {
        if let Ok(p) = std::env::var("MYPI_MODELS") {
            return Ok(std::path::PathBuf::from(p));
        }
        Ok(Self::xdg_config_base()?.join("mypi").join("models.yml"))
    }

    /// config.yaml path: $MYPI_CONFIG overrides, else $XDG_CONFIG_HOME/mypi/config.yaml.
    pub fn app_path() -> anyhow::Result<std::path::PathBuf> {
        if let Ok(p) = std::env::var("MYPI_CONFIG") {
            return Ok(std::path::PathBuf::from(p));
        }
        Ok(Self::xdg_config_base()?.join("mypi").join("config.yaml"))
    }

    /// Directory containing the config files. Used by saves.
    pub fn config_dir() -> anyhow::Result<std::path::PathBuf> {
        Ok(Self::xdg_config_base()?.join("mypi"))
    }

    /// Load both files.
    ///
    /// - models.yml missing -> **hard error** (the user must declare their
    ///   providers/models/keys; nothing is invented on their behalf).
    /// - config.yaml missing -> bootstrapped: the models are sorted by
    ///   display name (A-Z) and the first becomes `default`; the file is
    ///   written so subsequent /model changes persist. models.yml present
    ///   but empty (no models) is also a hard error.
    pub fn load() -> anyhow::Result<Self> {
        let models_path = Self::models_path()?;
        let app_path = Self::app_path()?;

        const TEMPLATE_URL: &str =
            "https://raw.githubusercontent.com/yanyjb1/MyPi/main/models_example.yml";
        let models_text = std::fs::read_to_string(&models_path).with_context(|| {
            format!(
                "no models.yml at {} — create it yourself (the program never writes this file):\n  1. get the template: {}\n  2. put it at {}, fill in your providers and keys",
                models_path.display(),
                TEMPLATE_URL,
                models_path.display()
            )
        })?;
        let models: ModelsConfig = serde_yaml::from_str(&models_text)
            .with_context(|| format!("invalid models.yml format: {}", models_path.display()))?;

        let app: AppConfig = match std::fs::read_to_string(&app_path) {
            Ok(text) => serde_yaml::from_str(&text)
                .with_context(|| format!("invalid config.yaml format: {}", app_path.display()))?,
            Err(_) => {
                // Bootstrap: pick the alphabetically-first display name and
                // persist the choice (config.yaml is program-owned; safe
                // to write — unlike models.yml, which is never written).
                let first = Self::alphabetical_default(&models).ok_or_else(|| {
                    anyhow!("models.yml declares no models — add at least one `- id: ...` entry")
                })?;
                let app = AppConfig {
                    default: Some(first),
                    theme: Theme::default(),
                };
                std::fs::create_dir_all(Self::config_dir()?)?;
                let yaml = serde_yaml::to_string(&app)?;
                std::fs::write(&app_path, yaml)
                    .with_context(|| format!("failed to write {}", app_path.display()))?;
                app
            }
        };

        let cfg = Config { models, app };
        cfg.validate()?;
        Ok(cfg)
    }

    /// The alphabetically-first model by display name (A-Z, case
    /// insensitive), addressed as `<provider>:<id>`. The deterministic
    /// choice when config.yaml has no default yet.
    fn alphabetical_default(models: &ModelsConfig) -> Option<String> {
        let mut all: Vec<(String, String, String)> = models
            .providers
            .iter()
            .flat_map(|(p, prov)| {
                prov.models
                    .iter()
                    .map(move |m| (m.display_name().to_lowercase(), p.clone(), m.id.clone()))
            })
            .collect();
        all.sort();
        all.first().map(|(_, p, id)| format!("{p}:{id}"))
    }

    /// Fail fast: model ids are **provider-scoped** (the same id may
    /// appear under two providers), so uniqueness is checked per provider.
    fn validate(&self) -> anyhow::Result<()> {
        let mut any = false;
        for (pname, p) in &self.models.providers {
            let mut seen = std::collections::HashSet::new();
            for m in &p.models {
                any = true;
                if !seen.insert(&m.id) {
                    return Err(anyhow!("provider {pname}: duplicate model id `{}`", m.id));
                }
            }
        }
        if !any {
            return Err(anyhow!(
                "models.yml declares no models — add at least one `- id: ...` entry"
            ));
        }
        // A persisted default must still address a declared model.
        if let Some(d) = &self.app.default {
            self.model_by_id(d)?;
        }
        Ok(())
    }

    /// Write the default model to config.yaml (never to models.yml).
    pub fn save_default(&self, id: &str) -> anyhow::Result<()> {
        let mut app = self.app.clone();
        app.default = Some(id.to_string());
        let path = Self::app_path()?;
        std::fs::create_dir_all(Self::config_dir()?)?;
        let yaml = serde_yaml::to_string(&app)?;
        std::fs::write(&path, yaml)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    /// Every declared model with its owning provider name, in config
    /// order (providers are a map, so sort names for a stable listing).
    pub fn models(&self) -> impl Iterator<Item = (&str, &ModelEntry)> {
        let mut names: Vec<&String> = self.models.providers.keys().collect();
        names.sort();
        names.into_iter().flat_map(move |n| {
            self.models.providers[n.as_str()]
                .models
                .iter()
                .map(move |m| (n.as_str(), m))
        })
    }

    /// Fetch the default model entry. The `default:` key is **mandatory**
    /// — declared by the user in the config, never inferred (no
    /// first-model fallback, no implicit default).
    pub fn default_model(&self) -> anyhow::Result<ResolvedModel> {
        let id = self
            .app
            .default
            .as_deref()
            .ok_or_else(|| anyhow!("no default model set — run /model <provider>:<id>"))?;
        self.model_by_id(id)
    }

    /// Find a model by its globally-addressed id `<provider>:<id>`
    /// (the form used by `default`, /model, /switch and completion).
    pub fn model_by_id(&self, id: &str) -> anyhow::Result<ResolvedModel> {
        let (pname, mid) = id
            .split_once(':')
            .ok_or_else(|| anyhow!("model id must be `<provider>:<id>`, got `{id}`"))?;
        let p = self
            .models
            .providers
            .get(pname)
            .ok_or_else(|| anyhow!("unknown provider: {pname}"))?;
        let m = p
            .models
            .iter()
            .find(|m| m.id == mid)
            .ok_or_else(|| anyhow!("provider {pname} has no model `{mid}`"))?;
        Ok(ResolvedModel {
            provider_name: pname.to_string(),
            entry: m.clone(),
        })
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

    fn parse_models(yaml: &str) -> anyhow::Result<ModelsConfig> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    #[test]
    fn parses_nested_provider_models() {
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://localhost:9999/v1
    api: openai-completions
    apiKey: ""
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
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig {
                default: Some("local:vendor-a/model-x".into()),
                theme: Theme::default(),
            },
        };
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

    /// `api` is required — a provider without it fails to parse.
    #[test]
    fn api_field_is_required() {
        let r = parse_models(
            r#"
providers:
  local: { baseUrl: "http://x/v1" }
"#,
        );
        assert!(r.is_err(), "missing api must be rejected");
    }

    /// A model id may itself contain colons (`global:x` — legacy vendor
    /// prefixes live inside the id). Addressing splits on the **first**
    /// colon only.
    #[test]
    fn model_id_may_contain_colons() {
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://x/v1
    api: openai-completions
    models:
      - id: global:gpt-5.6-luna
        name: GPT5.6L
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig {
                default: Some("local:global:gpt-5.6-luna".into()),
                theme: Theme::default(),
            },
        };
        let rm = cfg.default_model().unwrap();
        assert_eq!(rm.provider_name, "local");
        assert_eq!(rm.entry.id, "global:gpt-5.6-luna");
        // The wire id sent to the server keeps the colon.
        assert_eq!(rm.entry.id.split(':').count(), 2);
    }

    /// `default:` is mandatory — no first-model fallback at request time.
    #[test]
    fn default_is_mandatory() {
        let models = parse_models(
            r#"
providers:
  local: { baseUrl: "http://x/v1", api: openai-completions, models: [{ id: m1 }] }
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig::default(),
        };
        let err = cfg.default_model().unwrap_err().to_string();
        assert!(err.contains("no default model set"), "{err}");
    }

    /// Alphabetical bootstrap: config.yaml missing + models present ->
    /// the first model by display name (A-Z) becomes default and the
    /// file is written.
    #[test]
    fn alphabetical_default_bootstrap() {
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://x/v1
    api: openai-completions
    models:
      - id: m-zeta
        name: Zeta
      - id: m-alpha
        name: Alpha
      - id: m-bare
"#,
        )
        .unwrap();
        // "m-bare" (no name -> id fallback) sorts before "Alpha"? No:
        // id fallback "m-bare" lowercase m vs "alpha"/"zeta" — sorted:
        // alpha < m-bare < zeta. So "local:m-alpha" wins.
        let got = Config::alphabetical_default(&models).unwrap();
        assert_eq!(got, "local:m-alpha");
    }

    /// The theme section parses named/hex/r,g,b colors; defaults green/yellow/true black.
    #[test]
    fn theme_colors_parse() {
        use ratatui::style::Color;
        let app: AppConfig = serde_yaml::from_str(
            r##"
theme:
  accent: "#ff8800"
  gold: "12,34,56"
"##,
        )
        .unwrap();
        assert_eq!(app.theme.accent.to_color(), Color::Rgb(0xff, 0x88, 0x00));
        assert_eq!(app.theme.gold.to_color(), Color::Rgb(12, 34, 56));
        // Unconfigured black falls back to true black
        assert_eq!(app.theme.black.to_color(), Color::Rgb(0, 0, 0));

        // Everything defaulted
        let app: AppConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(app.theme.accent.to_color(), Color::Green);
        assert_eq!(app.theme.gold.to_color(), Color::Yellow);
        assert_eq!(app.theme.black.to_color(), Color::Rgb(0, 0, 0));
    }

    /// Cross-test mutex: tests that mutate environment variables share this lock.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Round-trip: /model persists the default into config.yaml only.
    #[test]
    fn save_default_writes_app_file_not_models() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-save2-{}", std::process::id()));
        unsafe {
            std::env::set_var("MYPI_CONFIG", dir.join("config.yaml"));
            std::env::set_var("MYPI_MODELS", dir.join("models.yml"));
        }
        std::fs::create_dir_all(&dir).unwrap();
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://x/v1
    api: openai-completions
    models:
      - id: a
      - id: b
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig {
                default: Some("local:a".into()),
                theme: Theme::default(),
            },
        };
        cfg.save_default("local:b").unwrap();
        let text = std::fs::read_to_string(dir.join("config.yaml")).unwrap();
        let back: AppConfig = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.default.as_deref(), Some("local:b"));
        // models.yml was never written by the program.
        assert!(!dir.join("models.yml").exists());
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
            std::env::remove_var("MYPI_MODELS");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Missing models.yml is a hard error pointing at the template.
    #[test]
    fn load_errors_when_models_missing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-missing2-{}", std::process::id()));
        unsafe {
            std::env::set_var("MYPI_CONFIG", dir.join("config.yaml"));
            std::env::set_var("MYPI_MODELS", dir.join("models.yml"));
        }
        let err = Config::load().unwrap_err().to_string();
        assert!(err.contains("create it yourself"), "{err}");
        assert!(err.contains("raw.githubusercontent.com"), "{err}");
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
            std::env::remove_var("MYPI_MODELS");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// models.yml present + config.yaml missing -> bootstrap writes
    /// config.yaml with the alphabetical default; no models -> error.
    #[test]
    fn bootstrap_writes_config_when_models_present() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-boot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("MYPI_CONFIG", dir.join("config.yaml"));
            std::env::set_var("MYPI_MODELS", dir.join("models.yml"));
        }
        std::fs::write(
            dir.join("models.yml"),
            r#"
providers:
  local:
    baseUrl: http://x/v1
    api: openai-completions
    models:
      - id: m2
        name: Beta
      - id: m1
        name: Alpha
"#,
        )
        .unwrap();
        let cfg = Config::load().unwrap();
        assert_eq!(cfg.app.default.as_deref(), Some("local:m1"));
        // And the bootstrapped config file exists on disk.
        assert!(dir.join("config.yaml").exists());
        // Empty models list -> hard error.
        std::fs::write(
            dir.join("models.yml"),
            "providers:\n  local:\n    baseUrl: http://x/v1\n    api: openai-completions\n",
        )
        .unwrap();
        let err = Config::load().unwrap_err().to_string();
        assert!(err.contains("declares no models"), "{err}");
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
            std::env::remove_var("MYPI_MODELS");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
