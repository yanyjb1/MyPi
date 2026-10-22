//! The `config.` area of the configuration: everything `config.yaml` carries, split into `server`
//! and `tui`. Only the types live here.

use serde::{Deserialize, Serialize};

use super::model::Currency;

/// The shape of `config.yaml`: everything lives under the literal `config` key, and there is no
/// `#[serde(default)]` on that field — a file with no `config:` section is an error.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ConfigFile {
    pub config: AppConfig,
}

/// Everything `config.yaml` carries.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AppConfig {
    /// What the server side reads.
    pub server: ServerConfig,
    /// What every front end reads.
    pub ui: UiConfig,
    /// What the terminal front end reads on its own.
    pub tui: TuiConfig,
}

// ---- server: ----

/// `server:` — the server side of the configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub api: ApiConfig,
    pub request: RequestConfig,
    pub agent: AgentConfig,
    pub storage: StorageConfig,
    pub platform: PlatformConfig,
    /// On a rewind, drop what follows or keep it as a branch you can return to. Only the dropping
    /// mode is built: `false` is refused when the config loads.
    #[serde(default = "d_prune_on_rewind")]
    pub prune_on_rewind: bool,
}

fn d_prune_on_rewind() -> bool {
    true
}

/// `config.server.api:` — how the server addresses a model.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiConfig {
    /// Default model, addressed as `<provider>:<model id>`. `None` = the alphabetically-first
    /// model of `models.yml`.
    pub default_model: Option<String>,
}

/// `config.server.request:` — how a request is sent and how long the context may be.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RequestConfig {
    /// Stream delivery mode. See [`StreamMode`].
    pub streaming: StreamMode,
    /// Context-compaction knobs.
    pub compact: CompactConfig,
}

/// `config.server.agent:` — the loop, its tools, and which profile it runs.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentConfig {
    /// Active system-prompt profile (`/profile` switches it; a restart returns
    /// to this one). `None` = the built-in `oh-my-pi`.
    pub profile: Option<String>,
    /// May the profile's tool roster be re-read **after a compaction**?
    pub reload_on_compaction: bool,
    /// The loop itself.
    pub r#loop: LoopConfig,
    /// Tool-layer knobs.
    pub tools: ToolsConfig,
}

/// `config.server.agent.loop:` — how far the loop may run on its own.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoopConfig {
    /// Fence: how many tool-call rounds one round may run before the loop stops asking for tools.
    /// `false` turns it off; the built-in 20000 exists so this stays a fence, not a budget.
    pub max_tool_calls: Fence,
}

/// A limit that can also be switched off: a number, or `false`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Fence {
    /// `false` = no fence.
    Off(bool),
    /// A number = that many tool-call rounds.
    Limit(usize),
}

impl Fence {
    /// The fence as the loop wants it: `None` = off.
    pub fn limit(self) -> Option<usize> {
        match self {
            Fence::Off(_) => None,
            Fence::Limit(n) => Some(n),
        }
    }
}

impl Default for Fence {
    fn default() -> Self {
        Fence::Limit(20_000)
    }
}

/// `config.server.storage:` — how writes are batched.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StorageConfig {
    /// Land one batch every N completed tool calls inside a round. A round's
    /// **end** always lands, whatever this says.
    pub batch_tools: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self { batch_tools: 10 }
    }
}

/// `config.server.platform:` — what the operating system answers for us.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlatformConfig {
    /// Time zone by name (e.g. `Asia/Shanghai`). `None` = whatever the system
    /// says; a value here wins over the environment.
    pub timezone: Option<String>,
    /// Timestamp injection (instant-messaging mode).
    pub stamp: StampConfig,
    /// How the two processes talk across the boundary.
    #[serde(default)]
    pub ipc: IpcCost,
}

/// `config.server.platform.ipc:` — which cross-process endpoint to use. `Os` = whatever this
/// system does; the other two pin one for debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcCost {
    /// Whatever this system does.
    #[default]
    Os,
    /// A unix socket.
    Unix,
    /// A Windows named pipe.
    Pipe,
}

impl IpcCost {
    /// The endpoint this setting asks for on the system we are running on.
    pub fn cost(self) -> crate::server::platform::ipc::Cost {
        match self {
            IpcCost::Os => crate::server::platform::ipc::Cost::of_this_system(),
            IpcCost::Unix => crate::server::platform::ipc::Cost::Unix,
            IpcCost::Pipe => crate::server::platform::ipc::Cost::Pipe,
        }
    }
}

// ---- ui: ----

/// `config.ui:` — what every front end reads, the terminal one included.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiConfig {
    /// The session layer's own switches.
    pub session: SessionConfig,
    /// The words a front end draws where the server stored a placeholder
    /// name. See [`PlaceholderConfig`].
    pub placeholder: PlaceholderConfig,
}

/// `config.ui.session:` — the session layer's own switches.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionConfig {
    /// How many blocks the content cache keeps around the view.
    #[serde(default = "d_preload_cache")]
    pub preload_cache: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            preload_cache: d_preload_cache(),
        }
    }
}

// ---- tui: ----

/// `config.tui:` — the terminal front end's own switches.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TuiConfig {
    /// Palette.
    pub theme: Theme,
    /// How many blocks of rendered lines to keep around the viewport.
    pub render_margin: usize,
    /// How the task list folds.
    pub todo_fold: TodoFold,
    /// `/usage` panel display options.
    pub usage: UsageConfig,
    /// Exchange rate.
    pub fx: FxConfig,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            render_margin: d_render_margin(),
            todo_fold: TodoFold::default(),
            usage: UsageConfig::default(),
            fx: FxConfig::default(),
        }
    }
}

/// `config.ui.placeholder:` — the text for each placeholder **name** the server
/// stores. The database keeps the name, the front end owns the words.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaceholderConfig {
    /// An assistant reply that arrived empty.
    pub empty_reply: String,
    /// A round the user actually interrupted.
    pub interrupted: String,
    /// A rewind picked a user message: those words are on the clipboard.
    pub copied_to_clipboard: String,
}

impl Default for PlaceholderConfig {
    fn default() -> Self {
        Self {
            empty_reply: "SYSTEM: (no reply body — empty response)".into(),
            interrupted: "SYSTEM: UserInterrupted.".into(),
            copied_to_clipboard: "SYSTEM: that message is on your clipboard.".into(),
        }
    }
}

// ---- server.agent.tools: ----

/// `config.server.agent.tools:` — how the tool executor runs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolsConfig {
    /// Cap on one `bash` command, in seconds. Default 600 (10 minutes).
    pub bash_timeout: u64,
    /// How much of one tool result the conversation keeps inline, in lines.
    pub output_max_lines: usize,
    /// The same cap in bytes — for results that are few lines but huge
    /// (minified JSON, base64 blobs, one 40 MB line).
    pub output_max_bytes: usize,
    /// Which browser the web tools drive.
    pub browser: BrowserConfig,
    /// Limits of the web tools.
    pub web: WebConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            bash_timeout: 600,
            output_max_lines: 512,
            output_max_bytes: 256 * 1024,
            browser: BrowserConfig::default(),
            web: WebConfig::default(),
        }
    }
}

impl ToolsConfig {
    /// The inline-output budget as the tool layer consumes it.
    pub fn limits(&self) -> OutputLimits {
        OutputLimits::from(self)
    }
}

/// The inline-output budget, as the tools layer consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLimits {
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        let t = ToolsConfig::default();
        Self {
            max_lines: t.output_max_lines,
            max_bytes: t.output_max_bytes,
        }
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

/// `config.server.agent.tools.browser:` — which browser the web tools drive.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrowserConfig {
    /// Chromium-family executable to launch when nothing is listening.
    /// `None` = `$MYPI_BROWSER_BIN`, else a shipped default path.
    pub bin: Option<String>,
    /// Attach to an already-running browser on this port instead of launching
    /// one (the long-lived-session mode).
    pub port: Option<u16>,
    /// Persistent profile directory: cookies, logins and extensions survive
    /// restarts.
    pub profile_dir: Option<std::path::PathBuf>,
    /// Keep one browser profile across runs, logins and all.
    pub persist_profile: bool,
    /// Launch headless. Default true: nobody is watching a window.
    pub headless: bool,
    /// Override the User-Agent of the launched browser.
    pub user_agent: Option<String>,
    /// Extra argv appended to the launch command (`--proxy-server=…`, …).
    pub extra_args: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            bin: None,
            port: None,
            profile_dir: None,
            persist_profile: true,
            headless: true,
            user_agent: None,
            extra_args: Vec::new(),
        }
    }
}

/// `config.server.agent.tools.web:` — how much of a page the web tools may hand back.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebConfig {
    /// Characters kept from a fetched page.
    pub fetch_chars: usize,
    /// Characters kept from a browser read.
    pub browser_chars: usize,
    /// How many search hits come back.
    pub search_hits: usize,
    /// Bytes fetched at most for one search result page.
    pub page_bytes: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            fetch_chars: 24_000,
            browser_chars: 24_000,
            search_hits: 20,
            page_bytes: 2 * 1024 * 1024,
        }
    }
}

// ---- the rest of the sections ----

/// Palette: loaded from `config.tui.theme`. No colors are hardcoded in code.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Theme {
    /// JSON theme name (bundled, or a file under `~/.config/mypi/themes`). When set, the JSON theme
    /// wins and the four colors below only fill the gaps.
    pub name: Option<String>,
    /// Borders, model name, used context.
    pub accent: ColorSpec,
    /// The money color.
    pub gold: ColorSpec,
    /// The capsule background — true black by default, which fixes Konsole's
    /// grayish indexed black.
    pub black: ColorSpec,
    /// The secondary gray (the π symbol and friends).
    pub muted: ColorSpec,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: None,
            accent: ColorSpec::Text("green".into()),
            gold: ColorSpec::Text("yellow".into()),
            black: ColorSpec::Rgb(0, 0, 0),
            muted: ColorSpec::Text("darkgray".into()),
        }
    }
}

/// Color syntax: a named color (`green`, `blue`, …), `#rrggbb`, or `r,g,b`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ColorSpec {
    Rgb(u8, u8, u8),
    Text(String),
}

impl ColorSpec {
    /// The original textual spec (the theme override detector compares these).
    pub fn to_spec(&self) -> String {
        match self {
            ColorSpec::Rgb(r, g, b) => format!("{r},{g},{b}"),
            ColorSpec::Text(s) => s.clone(),
        }
    }
}

/// `config.server.request.compact:` — context-compaction knobs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompactConfig {
    /// Verbatim tail budget (tokens, approximated). Small values make debugging
    /// cheap; the default is the full-fat 20k.
    pub retain_tail: usize,
    /// The instruction file to use when the profile does not bring its own.
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

/// `config.server.platform.stamp:` — timestamp injection (instant-messaging mode). It only
/// touches **new** user messages: the templates freeze into the message when it is written.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StampConfig {
    /// Off by default: not a byte is injected.
    pub enabled: bool,
    /// Template placed before the user's text.
    pub prefix: String,
    /// Template placed after the user's text.
    pub suffix: String,
}

impl Default for StampConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            prefix: "Last Time You Replied: {last}\n".into(),
            suffix: "\nSent From {now}".into(),
        }
    }
}

/// How the reply reaches the front end: chunk by chunk, or one finished block at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Every decoded chunk, the moment it arrives.
    #[default]
    Immediate,
    /// One piece per finished block: half-written text never goes out.
    Buffered,
}

/// `config.tui.todoFold` — how the task list folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TodoFold {
    /// `true` = fold, with the default cap.
    On(bool),
    /// `N` = fold, at most N items.
    Capped(usize),
}

impl TodoFold {
    /// Rows shown per phase when folded.
    pub const DEFAULT_CAP: usize = 8;

    /// Is folding on? `false` and `0` are both off.
    pub fn is_on(self) -> bool {
        match self {
            TodoFold::On(on) => on,
            TodoFold::Capped(n) => n > 0,
        }
    }

    /// Rows shown per phase (off = no cap).
    pub fn cap(self) -> usize {
        match self {
            TodoFold::On(true) => Self::DEFAULT_CAP,
            TodoFold::On(false) => usize::MAX,
            TodoFold::Capped(0) => usize::MAX,
            TodoFold::Capped(n) => n,
        }
    }
}

impl Default for TodoFold {
    fn default() -> Self {
        TodoFold::On(false)
    }
}

fn d_render_margin() -> usize {
    64
}

fn d_preload_cache() -> usize {
    128
}

/// `config.tui.usage:` — `/usage` panel display options.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageConfig {
    /// Where the panel sits.
    pub align: UsageAlign,
}

/// Horizontal position of a panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageAlign {
    #[default]
    Center,
    Left,
    Right,
}

/// `config.tui.fx:` — the rate used to fold bills of different currencies into one. Display only:
/// nothing here takes part in pricing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FxConfig {
    /// 1 USD in CNY. Default 6.
    pub usd_cny: f64,
}

impl Default for FxConfig {
    fn default() -> Self {
        Self { usd_cny: 6.0 }
    }
}

impl FxConfig {
    /// Fold one amount into CNY (the bill's unit) for display.
    pub fn to_display(&self, amount: f64, from: Currency) -> (f64, &'static str) {
        match from {
            Currency::Cny | Currency::Credits => (amount, from.symbol()),
            Currency::Usd => (amount * self.usd_cny, Currency::Cny.symbol()),
        }
    }
}
