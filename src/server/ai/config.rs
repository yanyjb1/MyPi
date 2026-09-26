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

/// **config.yaml → `tools:`** — tool-layer knobs: how the executor runs, as
/// opposed to model/provider configuration.
///
/// It lives in this module (not `agent::tools`) because the config layer must
/// stay a **leaf**: it used to name `agent::tools::ToolsConfig` and
/// `server::compaction::CompactConfig` directly, so the bottom layer depended
/// on two layers above it. Consumers import these downward now.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolsConfig {
    /// Cap on one `bash` command, in seconds. Default 600 (10 min).
    /// Accepts `bashTimeoutSecs` (camelCase, the file's prevailing style)
    /// or `bash_timeout_secs`.
    #[serde(
        default = "d_bash_timeout",
        rename = "bashTimeoutSecs",
        alias = "bash_timeout_secs"
    )]
    pub bash_timeout_secs: u64,
    /// How much of one tool result the conversation keeps inline, in lines.
    /// Above this the result becomes a 巨物 (artifact) and the model gets a
    /// placeholder; `read` stops at the same number and points at the rest.
    /// One pair of numbers, two uses: "the size at which output stops being
    /// part of the conversation" is a property of the conversation, not of
    /// whichever tool produced the bytes.
    #[serde(
        default = "d_output_max_lines",
        rename = "outputMaxLines",
        alias = "output_max_lines"
    )]
    pub output_max_lines: usize,
    /// The same cap in bytes — for results that are few lines but huge
    /// (minified JSON, base64 blobs, a single 40 MB line).
    #[serde(
        default = "d_output_max_bytes",
        rename = "outputMaxBytes",
        alias = "output_max_bytes"
    )]
    pub output_max_bytes: usize,
    /// May the profile's tool roster change **after a compaction**?
    ///
    /// The roster is read once, before the first turn. That is the only moment
    /// it can change without rewriting history: a tool appearing mid-conversation
    /// invalidates the provider's prefix cache and leaves the model holding
    /// results from a tool it can no longer see. A compaction is the other
    /// legal seam — the conversation is being rewritten anyway — so this knob
    /// (default `false`, i.e. the roster is fixed for the session's life) opts
    /// into re-reading the profile there.
    #[serde(default, rename = "reloadOnCompaction", alias = "reload_on_compaction")]
    pub reload_on_compaction: bool,
}

fn d_bash_timeout() -> u64 {
    600
}

/// 512 lines ≈ a screenful of screens.
fn d_output_max_lines() -> usize {
    crate::server::agent::artifacts::SPILL_LINES
}

/// 256 KiB guards the few-huge-lines case.
fn d_output_max_bytes() -> usize {
    crate::server::agent::artifacts::SPILL_BYTES
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            bash_timeout_secs: d_bash_timeout(),
            output_max_lines: d_output_max_lines(),
            output_max_bytes: d_output_max_bytes(),
            reload_on_compaction: false,
        }
    }
}

/// The inline-output budget, as the tools layer consumes it.
///
/// A plain pair of numbers rather than a reference to [`ToolsConfig`]: the
/// executor only needs these two, and naming the config type from the tool
/// layer would make the dependency look wider than it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLimits {
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            max_lines: d_output_max_lines(),
            max_bytes: d_output_max_bytes(),
        }
    }
}

impl ToolsConfig {
    /// The inline-output budget as the tool layer consumes it.
    pub fn limits(&self) -> OutputLimits {
        OutputLimits::from(self)
    }
}

impl From<&ToolsConfig> for OutputLimits {
    fn from(c: &ToolsConfig) -> Self {
        Self {
            max_lines: c.output_max_lines,
            max_bytes: c.output_max_bytes,
        }
    }
}

/// **config.yaml → `browser:`** — which browser the web tools drive.
///
/// Only meaningful with the `web` feature compiled in; the section is inert
/// otherwise. Environment variables still win over the file (they are the
/// escape hatch for tests and for parallel sessions):
/// `MYPI_BROWSER_BIN`, `MYPI_BROWSER_PORT`, `MYPI_BROWSER_PROFILE_DIR`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BrowserConfig {
    /// Chromium-family executable to launch when nothing is listening.
    /// None = `$MYPI_BROWSER_BIN`, else `/opt/helium/helium`.
    #[serde(default)]
    pub bin: Option<String>,
    /// Attach to an already-running browser on this port instead of launching
    /// one (the long-lived-session mode). None = reuse a live instance found
    /// on the profile, else launch.
    #[serde(default)]
    pub port: Option<u16>,
    /// Persistent profile directory (cookies, logins, extensions survive
    /// restarts). None = `$XDG_DATA_HOME/mypi/browser/profile`.
    ///
    /// Setting a path here implies `persistProfile` — an explicit directory is
    /// a request for the logins in it.
    #[serde(default, rename = "profileDir")]
    pub profile_dir: Option<std::path::PathBuf>,
    /// Keep one browser profile across runs, logins and all.
    ///
    /// Default `true`: the tool drives a real browser, and a real browser is
    /// signed in to the sites the user signed in to. Set it to `false` for a
    /// throwaway profile (fresh temp dir per process, removed on exit) when the
    /// session should see the logged-out web — or when two sessions must not
    /// share cookies. Only affects a browser we **launch**: attaching to an
    /// already-running one (`port`/`MYPI_BROWSER_PORT`) uses that browser's
    /// profile whatever this says.
    #[serde(default = "d_persist_profile", rename = "persistProfile")]
    pub persist_profile: bool,
    /// Launch headless. Default true: nobody is watching a window.
    #[serde(default = "d_headless")]
    pub headless: bool,
    /// Override the User-Agent of the launched browser. None = a plain
    /// stable-channel desktop UA (the headless UA is the loudest automation
    /// tell).
    #[serde(default, rename = "userAgent")]
    pub user_agent: Option<String>,
    /// Extra argv appended to the launch command (`--proxy-server=…`, …).
    #[serde(default, rename = "extraArgs")]
    pub extra_args: Vec<String>,
}

fn d_headless() -> bool {
    true
}

fn d_persist_profile() -> bool {
    true
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            bin: None,
            port: None,
            profile_dir: None,
            persist_profile: d_persist_profile(),
            headless: d_headless(),
            user_agent: None,
            extra_args: Vec::new(),
        }
    }
}

/// **config.yaml → `compact:`** — context-compaction knobs (see
/// `server::compaction` for what they do). Here for the same layering reason
/// as [`ToolsConfig`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct CompactConfig {
    /// Verbatim tail budget (tokens, approximated). Small values make
    /// debugging cheap; default is the full-fat 20k.
    pub retain_tail: usize,
    /// Optional external instruction file (read fresh at each compaction).
    pub instruction_file: Option<std::path::PathBuf>,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            retain_tail: 20_000,
            instruction_file: None,
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

/// How the reply **text** reaches subscribers (the TUI, a bridge, a web UI).
///
/// The wire is decoded once, in the server: subscribers receive **content**,
/// never JSON. This switch only decides *when* that content is handed over.
///
/// It is **not** a switch for the transcript: entries (your own message, each
/// tool call, each tool result, errors) are shipped block by block in both
/// modes — that is what the pump's tail reconciliation does. What changes is
/// only how a block *of text* arrives:
///
/// - [`Immediate`](StreamMode::Immediate) — as it is typed;
/// - [`Buffered`](StreamMode::Buffered) — whole, the moment that block's text
///   is done (a block's text is done when the model asks for a tool, or when
///   the turn ends).
///
/// So neither mode is "collect the turn and dump it at the end": buffered is
/// **block by block, not turn by turn**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Forward every decoded chunk the moment it arrives (typewriter output).
    #[default]
    Immediate,
    /// Hold a block's text and hand it over in one piece once that block's text
    /// is complete — when the model asks for a tool, or when the turn ends.
    ///
    /// Same final state, one emission per block instead of thousands. Two
    /// uses: a client that cannot render partial text (a chat bridge editing a
    /// single message), and debugging a trace you cannot read while it
    /// scrolls. A round that dies mid-flight still hands over what it had, so
    /// buffered and immediate differ in emission count, never in stored text.
    Buffered,
}

/// **config.yaml** — program-managed miscellany (default model, theme).
/// Safe to rewrite: the program owns this file.
// Parse a `MYPI_STREAM_MODE` value. Unknown input = no override (never a
// silent fallback to a default: a typo must not change behavior quietly).
fn stream_mode_override(raw: &str) -> Option<StreamMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "buffered" | "batch" => Some(StreamMode::Buffered),
        "immediate" | "stream" => Some(StreamMode::Immediate),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AppConfig {
    /// Default model id, addressed as `<provider>:<model id>`.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub compact: CompactConfig,
    /// Tool-layer knobs (`bash` timeout, ...). See [`ToolsConfig`].
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Which browser the web tools drive (only used with the `web` feature).
    /// See [`BrowserConfig`].
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Active system-prompt profile name (`/profile` switches it;
    /// restart returns to this default). None = `default`.
    #[serde(default)]
    pub profile: Option<String>,
    /// Stream delivery mode (`immediate` | `buffered`). See [`StreamMode`].
    #[serde(default)]
    pub streaming: StreamMode,
    /// 前端窗口的两个旋钮（内存的上界全在这里）。
    #[serde(default)]
    pub tui: TuiConfig,
}

/// **窗口化**的两个数（见 `tui::zone::main::history`）。
///
/// 历史区只留"看得见的那一段"：一段**已落盘**的连续窗口 + 还没落盘的活尾巴。
/// 这两个数决定窗口多大、渲染行缓存留多少块。它们是**深度**旋钮，不是省内存
/// 的旋钮：实测把渲染缓存从 256 块降到 32 块，进程 RSS 只差 ~4 MB。
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct TuiConfig {
    /// 渲染行缓存留多少块（视口上下各一份）。
    ///
    /// 也是滚动的手感旋钮：越大，往上翻时越少遇到没量过的块。
    #[serde(default = "d_render_margin", rename = "renderMargin")]
    pub render_margin: usize,
    /// 锚点上面预取/预保留多少**块**（再往上就该去库里取下一页了）。
    #[serde(default = "d_preload")]
    pub preload: usize,
}

fn d_render_margin() -> usize {
    64
}

fn d_preload() -> usize {
    128
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            render_margin: d_render_margin(),
            preload: d_preload(),
        }
    }
}

/// The merged view both files feed into (what the rest of the program sees).
#[derive(Debug, Clone)]
pub struct Config {
    pub models: ModelsConfig,
    pub app: AppConfig,
}

/// Connection details for one provider.
///
/// Note there is **no** `api:` key: it used to be required and fed a
/// `Dialect` accessor that nothing called, so a hand-maintained models.yml
/// without it could not start the program at all (the error was
/// `missing field api`). Files that still carry the key keep loading — serde
/// ignores unknown fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Provider {
    #[serde(alias = "baseUrl")]
    pub base_url: String,
    #[serde(default, alias = "apiKey")]
    pub api_key: String,
    /// Models this provider serves — **nested inside the provider**, never
    /// a top-level flat list. The model id is provider-internal: the same
    /// gateway can expose `gpt-x`, DeepSeek's API exposes
    /// `deepseek-chat`; ids are written per provider by the user.
    #[serde(default, rename = "models")]
    pub models: Vec<ModelEntry>,
}

/// A model plus the provider it lives under (the config nests models
/// inside providers; callers always need both).
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub entry: ModelEntry,
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

    /// Persist `id` as the default model in `config.yaml`, and return where it
    /// went.
    ///
    /// Resolved first: writing a model id that does not exist would break the
    /// user's *next* start, which is the one thing this must never do.
    ///
    /// The file is edited **by key**, textually, one line at a time: a YAML
    /// round-trip would rewrite the whole document and drop the user's comments
    /// and key order. When the config layer is reworked, this body changes and
    /// nothing else does — callers only know `CommandEnv::set_default_model`.
    pub fn set_default_model(&self, id: &str) -> anyhow::Result<std::path::PathBuf> {
        self.model_by_id(id)?;
        let path = Self::app_path()?;
        write_default_key(&path, id)?;
        Ok(path)
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
                    streaming: StreamMode::default(),
                    default: Some(first),
                    theme: Theme::default(),
                    compact: Default::default(),
                    tools: Default::default(),
                    browser: Default::default(),
                    profile: None,
                    tui: TuiConfig::default(),
                };
                std::fs::create_dir_all(Self::config_dir()?)?;
                let yaml = serde_yaml::to_string(&app)?;
                std::fs::write(&app_path, yaml)
                    .with_context(|| format!("failed to write {}", app_path.display()))?;
                app
            }
        };

        let mut cfg = Config { models, app };
        // Debug override: flip stream delivery without editing a file (the
        // TUI command layer is not wired to the session yet, and a test run
        // should not have to own a config.yaml).
        if let Ok(v) = std::env::var("MYPI_STREAM_MODE") {
            cfg.app.streaming = stream_mode_override(&v).unwrap_or(cfg.app.streaming);
        }
        Self::finish_load(cfg)
    }

    /// Validate the loaded config and make a stale `default` survivable.
    ///
    /// models.yml is **hand-edited**, so renaming a provider or model there can
    /// leave `config.yaml`'s persisted default pointing at something that no
    /// longer exists. Refusing to start would be a dead end: the failure happens
    /// before `/model` is reachable, so the only way out would be editing
    /// config.yaml by hand. Instead: warn, fall back to the deterministic
    /// alphabetical pick, and **leave the file alone** (`/model` is how the
    /// choice becomes permanent).
    fn finish_load(mut cfg: Config) -> anyhow::Result<Config> {
        cfg.validate()?;
        if let Some(stale) = cfg.app.default.clone()
            && let Err(e) = cfg.model_by_id(&stale)
        {
            let fallback = Self::alphabetical_default(&cfg.models);
            eprintln!(
                "mypi: 默认模型 {stale} 已无法解析（{e:#}）；本次改用 {}，用 /model 重新指定",
                fallback.as_deref().unwrap_or("(无)")
            );
            cfg.app.default = fallback;
        }
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
        // Structural only: a persisted default that no longer resolves is
        // handled by `finish_load` (warn + fall back), never here.
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


    
/// Write the top-level `default:` key of a config file, **textually**.
///
/// A YAML round-trip would rewrite the whole document and drop the user's
/// comments and key order; this touches one line and leaves every other byte
/// alone. The key (and the file) is created when missing.
fn write_default_key(path: &std::path::Path, id: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = String::with_capacity(text.len() + id.len() + 16);
    let mut replaced = false;
    for line in text.lines() {
        // 认**顶层** `default:` 这个键：缩进的 `default:` 属于某个小节，
        // 动它就是在改别人的配置。（不数行号——行号会随版本漂移。）
        let top_level = !line.starts_with([' ', '\t']);
        if !replaced && top_level && line.trim_start().starts_with("default:") {
            out.push_str(&format!("default: {id}"));
            replaced = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !replaced {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("default: {id}\n"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, out)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn parse_models(yaml: &str) -> anyhow::Result<ModelsConfig> {
        Ok(serde_yaml::from_str(yaml)?)
    }

    #[test]
    fn a_stale_default_falls_back_instead_of_bricking_startup() {
        // The failure this guards: models.yml gets a provider renamed by hand,
        // config.yaml's persisted default still names the old one, and startup
        // dies *before* /model is reachable — a dead end reachable only by
        // editing the file by hand.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("models.yml"),
            "providers:\n  local:\n    baseUrl: http://x/v1\n    models:\n      - id: m-alpha\n        name: Alpha\n",
        )
        .unwrap();
        let cfg_path = dir.join("config.yaml");
        std::fs::write(&cfg_path, "default: gone:whatever\n").unwrap();
        unsafe {
            std::env::set_var("MYPI_CONFIG", &cfg_path);
            std::env::set_var("MYPI_MODELS", dir.join("models.yml"));
        }

        let cfg = Config::load().unwrap();
        assert_eq!(
            cfg.app.default.as_deref(),
            Some("local:m-alpha"),
            "必须回落到确定性的选择，而不是拒绝启动"
        );
        // The user's file is left exactly as they wrote it.
        let text = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            text.contains("gone:whatever"),
            "不得回写用户的 default: {text}"
        );

        unsafe {
            std::env::remove_var("MYPI_CONFIG");
            std::env::remove_var("MYPI_MODELS");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tools_config_reads_camel_and_snake_and_defaults() {
        // The file's prevailing key style is camelCase; both spellings are
        // accepted, and an absent block falls back to 10 minutes.
        let camel: AppConfig = serde_yaml::from_str("tools:\n  bashTimeoutSecs: 42\n").unwrap();
        assert_eq!(camel.tools.bash_timeout_secs, 42);
        let snake: AppConfig = serde_yaml::from_str("tools:\n  bash_timeout_secs: 90\n").unwrap();
        assert_eq!(snake.tools.bash_timeout_secs, 90);
        let absent: AppConfig = serde_yaml::from_str("theme: {}\n").unwrap();
        assert_eq!(absent.tools.bash_timeout_secs, 600);
    }

    #[test]
    fn the_output_budget_comes_from_config_yaml() {
        // The numbers that decide "how much output stays part of the
        // conversation" are the user's, not the code's: one pair, used by both
        // the 巨物 spill and `read`'s default cap.
        let camel: AppConfig =
            serde_yaml::from_str("tools:\n  outputMaxLines: 40\n  outputMaxBytes: 2048\n").unwrap();
        assert_eq!(camel.tools.limits().max_lines, 40);
        assert_eq!(camel.tools.limits().max_bytes, 2048);
        let snake: AppConfig =
            serde_yaml::from_str("tools:\n  output_max_lines: 9\n  output_max_bytes: 100\n")
                .unwrap();
        assert_eq!(snake.tools.limits().max_lines, 9);
        assert_eq!(snake.tools.limits().max_bytes, 100);
        let absent: AppConfig = serde_yaml::from_str("theme: {}\n").unwrap();
        let d = crate::server::agent::artifacts::SPILL_LINES;
        assert_eq!(absent.tools.limits().max_lines, d, "缺省=巨物阈值");
    }

    #[test]
    fn the_roster_reload_knob_defaults_off() {
        // Changing the tool set mid-conversation is the thing that invalidates
        // the prefix cache and strands the model with results from a tool it
        // cannot see — so the seam after a compaction is opt-in.
        let on: AppConfig = serde_yaml::from_str("tools:\n  reloadOnCompaction: true\n").unwrap();
        assert!(on.tools.reload_on_compaction);
        let snake: AppConfig =
            serde_yaml::from_str("tools:\n  reload_on_compaction: true\n").unwrap();
        assert!(snake.tools.reload_on_compaction);
        let absent: AppConfig = serde_yaml::from_str("theme: {}\n").unwrap();
        assert!(!absent.tools.reload_on_compaction, "缺省=一个会话里名册固定");
    }

    #[test]
    fn the_stream_mode_comes_from_config_yaml_too() {
        // `app.streaming` 是用户能按的旋钮；写错的值不该悄悄变成默认值
        // （解析不出来=整份配置报错，人一眼看得见）。
        let buf: AppConfig = serde_yaml::from_str("streaming: buffered\n").unwrap();
        assert_eq!(buf.streaming, StreamMode::Buffered);
        let imm: AppConfig = serde_yaml::from_str("streaming: immediate\n").unwrap();
        assert_eq!(imm.streaming, StreamMode::Immediate);
        let absent: AppConfig = serde_yaml::from_str("theme: {}\n").unwrap();
        assert_eq!(absent.streaming, StreamMode::Immediate, "缺省=逐字");
        assert!(serde_yaml::from_str::<AppConfig>("streaming: buffer\n").is_err());
    }

    #[test]
    fn browser_config_comes_from_config_yaml() {
        // Which browser the web tools drive is a config-file decision, not a
        // hardcoded path: executable, port, profile, headless, UA, argv.
        let app: AppConfig = serde_yaml::from_str(
            r#"
browser:
  bin: /usr/bin/chromium
  port: 9222
  profileDir: /tmp/mypi-profile
  headless: false
  userAgent: "UA/1"
  extraArgs: ["--proxy-server=http://127.0.0.1:8080"]
"#,
        )
        .unwrap();
        assert_eq!(app.browser.bin.as_deref(), Some("/usr/bin/chromium"));
        assert_eq!(app.browser.port, Some(9222));
        assert_eq!(
            app.browser.profile_dir.as_deref(),
            Some(std::path::Path::new("/tmp/mypi-profile"))
        );
        assert!(!app.browser.headless, "显式 false 必须生效");
        assert_eq!(app.browser.user_agent.as_deref(), Some("UA/1"));
        // The login-persistence toggle: on by default (a real browser is signed
        // in), and switchable off for a throwaway profile.
        assert!(app.browser.persist_profile, "缺省=保留登录");
        let off: AppConfig =
            serde_yaml::from_str("browser:\n  persistProfile: false\n").unwrap();
        assert!(!off.browser.persist_profile);
        assert_eq!(
            app.browser.extra_args,
            vec!["--proxy-server=http://127.0.0.1:8080"]
        );

        // Absent block: headless, nothing pinned — the launcher then falls
        // back to $MYPI_BROWSER_BIN and the built-in UA.
        let app: AppConfig = serde_yaml::from_str("theme: {}\n").unwrap();
        assert!(app.browser.headless);
        assert!(app.browser.bin.is_none());
        assert!(app.browser.port.is_none());
        assert!(app.browser.user_agent.is_none());
    }

    #[test]
    fn parses_nested_provider_models() {
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://localhost:9999/v1
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
                tui: TuiConfig::default(),
                default: Some("local:vendor-a/model-x".into()),
                theme: Theme::default(),
                compact: Default::default(),
                tools: Default::default(),
                browser: Default::default(),
                profile: None,
                streaming: Default::default(),
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

    /// A provider needs only `baseUrl` (+ models) — no wire-dialect key.
    #[test]
    fn a_provider_without_the_wire_dialect_key_parses() {
        // `api:` used to be required, which made a hand-maintained models.yml
        // unable to start the program at all. It fed an accessor nothing called.
        let models = parse_models(
            r#"
providers:
  local: { baseUrl: "http://x/v1", models: [{ id: m1 }] }
"#,
        )
        .unwrap();
        assert_eq!(models.providers["local"].models.len(), 1);
    }

    /// Files that still carry the old `api:` key keep loading (unknown keys
    /// are ignored) — including the `deepseek` value, which never had a
    /// behavior behind it.
    #[test]
    fn a_legacy_wire_dialect_key_is_tolerated() {
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://x/v1
    api: deepseek
    models: [{ id: m1 }]
"#,
        )
        .unwrap();
        assert_eq!(models.providers["local"].models[0].id, "m1");
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
    models:
      - id: global:gpt-5.6-luna
        name: GPT5.6L
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig {
                tui: TuiConfig::default(),
                default: Some("local:global:gpt-5.6-luna".into()),
                theme: Theme::default(),
                compact: Default::default(),
                tools: Default::default(),
                browser: Default::default(),
                profile: None,
                streaming: Default::default(),
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
  local: { baseUrl: "http://x/v1", models: [{ id: m1 }] }
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

    /// /model 的落点：**只动 config.yaml 的一行**。用户的注释与键序必须原样
    /// 留下（这正是逐行改而不是 YAML 往返的原因），models.yml 一个字节都不写。
    #[test]
    fn set_default_model_rewrites_one_line_and_never_touches_models() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mypi-save2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        unsafe {
            std::env::set_var("MYPI_CONFIG", dir.join("config.yaml"));
            std::env::set_var("MYPI_MODELS", dir.join("models.yml"));
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.yaml"),
            "# 这是用户的注释，别动它\ndefault: local:a\ntheme:\n  accent: red\n",
        )
        .unwrap();
        let models = parse_models(
            r#"
providers:
  local:
    baseUrl: http://x/v1
    models:
      - id: a
      - id: b
"#,
        )
        .unwrap();
        let cfg = Config {
            models,
            app: AppConfig {
                tui: TuiConfig::default(),
                default: Some("local:a".into()),
                theme: Theme::default(),
                compact: Default::default(),
                tools: Default::default(),
                browser: Default::default(),
                profile: None,
                streaming: Default::default(),
            },
        };
        let path = cfg.set_default_model("local:b").unwrap();
        assert_eq!(path, dir.join("config.yaml"), "写的是 config.yaml");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("default: local:b"), "{text}");
        assert!(!text.contains("local:a"), "旧值要被换掉：{text}");
        assert!(text.contains("# 这是用户的注释，别动它"), "注释被吃了：{text}");
        assert!(text.contains("accent: red"), "别的键被吃了：{text}");
        // 未声明的 id 在写盘之前就被拒——绝不写坏下一份配置。
        assert!(cfg.set_default_model("local:zzz").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        // models.yml 是用户的文件，程序永远不写它。
        assert!(!dir.join("models.yml").exists());
        unsafe {
            std::env::remove_var("MYPI_CONFIG");
            std::env::remove_var("MYPI_MODELS");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 逐行改写的三条边界：换掉顶层那一行、键不存在就追加、缩进的同名键不碰。
    #[test]
    fn write_default_key_edits_the_top_level_key_only() {
        let dir = std::env::temp_dir().join(format!("mypi-wdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("a.yaml");
        std::fs::write(&path, "# c\ndefault: x\n").unwrap();
        write_default_key(&path, "y").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# c\ndefault: y\n");

        let path = dir.join("b.yaml");
        std::fs::write(&path, "theme: {}\n").unwrap();
        write_default_key(&path, "y").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "theme: {}\ndefault: y\n",
            "键不存在时追加，不动别的行"
        );

        let path = dir.join("c.yaml");
        std::fs::write(&path, "theme:\n  default: keep-me\n").unwrap();
        write_default_key(&path, "y").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("  default: keep-me"), "缩进的同名键是别人的：{text}");
        assert!(text.ends_with("default: y\n"), "{text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Missing models.yml is a hard error pointing at the template.
    #[test]
    fn the_default_model_is_written_without_touching_anything_else() {
        // config.yaml 是用户的文件：注释、键序、缩进都得原样留着——所以这条
        // 路是**改一行文本**，不是 YAML 往返（往返会把注释吃掉）。
        let dir = std::env::temp_dir().join(format!("mypi-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(
            &path,
            "# 我的配置\ntheme: {}\ndefault: fake:model-a\ntools:\n  bashTimeoutSecs: 42\n",
        )
        .unwrap();
        write_default_key(&path, "fake:model-b").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("default: fake:model-b"), "{after}");
        assert!(after.contains("# 我的配置"), "注释必须留着：{after}");
        assert!(after.contains("bashTimeoutSecs: 42"), "别的键必须留着：{after}");

        // 没有这个键就加上；文件不存在就建出来。
        std::fs::write(&path, "theme: {}\n").unwrap();
        write_default_key(&path, "fake:model-c").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("default: fake:model-c"));
        let fresh = dir.join("nested/config.yaml");
        write_default_key(&fresh, "fake:model-d").unwrap();
        assert!(std::fs::read_to_string(&fresh).unwrap().contains("default: fake:model-d"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
            "providers:\n  local:\n    baseUrl: http://x/v1\n",
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

#[cfg(test)]
mod stream_mode_tests {
    use super::{StreamMode, stream_mode_override};

    #[test]
    fn the_env_override_parses_and_refuses_typos() {
        // 调试开关：写错不许悄悄退回默认值，宁可不生效。
        assert_eq!(stream_mode_override("buffered"), Some(StreamMode::Buffered));
        assert_eq!(stream_mode_override(" BATCH\n"), Some(StreamMode::Buffered));
        assert_eq!(
            stream_mode_override("immediate"),
            Some(StreamMode::Immediate)
        );
        assert_eq!(stream_mode_override("stream"), Some(StreamMode::Immediate));
        assert_eq!(stream_mode_override("buffer"), None);
        assert_eq!(stream_mode_override(""), None);
    }
}
