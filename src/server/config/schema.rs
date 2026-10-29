//! The whole schema: one declaration per key, expanded into the table and constants.

use super::registry::{DefaultValue, Key, Kind, keys};

const NO_ARGS: &[&str] = &[];

pub(crate) const STREAMING: &[&str] = &["immediate", "buffered"];
pub(crate) const IPC_KINDS: &[&str] = &["os", "unix", "pipe"];
pub(crate) const USAGE_ALIGN: &[&str] = &["center", "left", "right"];

keys! {
    KEY_CONFIG_SERVER_API_DEFAULT_MODEL = "config", "server.api", "defaultModel", Text, DefaultValue::Null, "Model requests use unless the session says otherwise.", true;
    KEY_CONFIG_SERVER_REQUEST_STREAMING = "config", "server.request", "streaming", Text, DefaultValue::Text("immediate"), "immediate = every chunk; buffered = one piece per block.", true;
    KEY_CONFIG_SERVER_REQUEST_COMPACT_RETAIN_TAIL = "config", "server.request.compact", "retainTail", Int, DefaultValue::Int(20000), "Tokens of the tail a compaction keeps verbatim.", true;
    KEY_CONFIG_SERVER_REQUEST_COMPACT_INSTRUCTION_FILE = "config", "server.request.compact", "instructionFile", Text, DefaultValue::Null, "Compaction instruction file; a profile's compact.md wins.", true;
    KEY_CONFIG_SERVER_AGENT_RELOAD_ON_COMPACTION = "config", "server.agent", "reloadOnCompaction", Bool, DefaultValue::Bool(false), "Re-read the profile's tool list after a compaction.", true;
    KEY_CONFIG_SERVER_AGENT_LOOP_MAX_TOOL_CALLS = "config", "server.agent.loop", "maxToolCalls", BoolOrInt, DefaultValue::Int(20000), "Fence on tool-call rounds; false turns it off.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BASH_TIMEOUT = "config", "server.agent.tools", "bashTimeout", Int, DefaultValue::Int(600), "Cap on one bash command, in seconds.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_LINES = "config", "server.agent.tools", "outputMaxLines", Int, DefaultValue::Int(512), "Lines of one tool result kept inline.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_BYTES = "config", "server.agent.tools", "outputMaxBytes", Int, DefaultValue::Int(262144), "Bytes of one tool result kept inline.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_BIN = "config", "server.agent.tools.browser", "bin", Text, DefaultValue::Null, "Chromium-family executable; empty = $MYPI_BROWSER_BIN or a shipped default.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PORT = "config", "server.agent.tools.browser", "port", Int, DefaultValue::Null, "Attach to a running browser on this port instead of launching one.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PROFILE_DIR = "config", "server.agent.tools.browser", "profileDir", Text, DefaultValue::Null, "Persistent browser profile directory.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_HEADLESS = "config", "server.agent.tools.browser", "headless", Bool, DefaultValue::Bool(true), "Launch the browser headless.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_USER_AGENT = "config", "server.agent.tools.browser", "userAgent", Text, DefaultValue::Null, "Override the launched browser's User-Agent.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_EXTRA_ARGS = "config", "server.agent.tools.browser", "extraArgs", List, DefaultValue::List(NO_ARGS), "Extra argv appended to the browser launch.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_CONCURRENT = "config", "server.agent.tools.kairos", "maxConcurrent", Int, DefaultValue::Int(4), "Background jobs allowed at once in one session.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_TIMEOUT_SECS = "config", "server.agent.tools.kairos", "timeoutSecs", Int, DefaultValue::Int(3600), "Default background-job timeout.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_OUTPUT_BYTES = "config", "server.agent.tools.kairos", "maxOutputBytes", Int, DefaultValue::Int(8388608), "Bytes captured per background job.", true;
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_RETAINED = "config", "server.agent.tools.kairos", "maxRetained", Int, DefaultValue::Int(8), "Completed unread results kept per session.", true;
    KEY_CONFIG_SERVER_AGENT_SUBAGENT_MAX_TOOL_CALLS = "config", "server.agent.subagent", "maxToolCalls", Int, DefaultValue::Int(1000), "Non-final tool calls one child may make.", true;
    KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS = "config", "server.storage", "batchTools", Int, DefaultValue::Int(10), "Completed tool calls per write batch.", true;
    KEY_CONFIG_SERVER_PLATFORM_TIMEZONE = "config", "server.platform", "timezone", Text, DefaultValue::Null, "Time zone by name; empty = the system's.", true;
    KEY_CONFIG_SERVER_PLATFORM_STAMP_ENABLED = "config", "server.platform.stamp", "enabled", Bool, DefaultValue::Bool(false), "Inject timestamps into new user messages.", true;
    KEY_CONFIG_SERVER_PLATFORM_STAMP_PREFIX = "config", "server.platform.stamp", "prefix", Text, DefaultValue::Text("Last Time You Replied: {last}\n"), "Template placed before the user's text.", true;
    KEY_CONFIG_SERVER_PLATFORM_STAMP_SUFFIX = "config", "server.platform.stamp", "suffix", Text, DefaultValue::Text("\nSent From {now}"), "Template placed after the user's text.", true;
    KEY_CONFIG_SERVER_PLATFORM_IPC = "config", "server.platform", "ipc", Text, DefaultValue::Text("os"), "Cross-process endpoint: os, unix or pipe.", true;
    KEY_CONFIG_SERVER_PRUNE_ON_REWIND = "config", "server", "pruneOnRewind", Bool, DefaultValue::Bool(true), "On a rewind, drop what follows; only true is built.", true;
    KEY_CONFIG_UI_SESSION_PRELOAD_CACHE = "config", "ui.session", "preloadCache", Int, DefaultValue::Int(128), "Blocks kept in the session content cache.", true;
    KEY_CONFIG_UI_PLACEHOLDER_EMPTY_REPLY = "config", "ui.placeholder", "emptyReply", Text, DefaultValue::Text("SYSTEM: (no reply body — empty response)"), "Words for an assistant reply that arrived empty.", true;
    KEY_CONFIG_UI_PLACEHOLDER_INTERRUPTED = "config", "ui.placeholder", "interrupted", Text, DefaultValue::Text("SYSTEM: UserInterrupted."), "Words for a round the user interrupted.", true;
    KEY_CONFIG_UI_PLACEHOLDER_COPIED_TO_CLIPBOARD = "config", "ui.placeholder", "copiedToClipboard", Text, DefaultValue::Text("SYSTEM: that message is on your clipboard."), "Words for a message copied by a rewind.", true;
    KEY_CONFIG_TUI_THEME_NAME = "config", "tui.theme", "name", Text, DefaultValue::Null, "JSON theme name; fills the four colors when set.", true;
    KEY_CONFIG_TUI_THEME_ACCENT = "config", "tui.theme", "accent", Text, DefaultValue::Text("green"), "Borders, model name and used context.", true;
    KEY_CONFIG_TUI_THEME_GOLD = "config", "tui.theme", "gold", Text, DefaultValue::Text("yellow"), "The money color.", true;
    KEY_CONFIG_TUI_THEME_BLACK = "config", "tui.theme", "black", Text, DefaultValue::Text("0,0,0"), "The capsule background.", true;
    KEY_CONFIG_TUI_THEME_MUTED = "config", "tui.theme", "muted", Text, DefaultValue::Text("darkgray"), "The secondary gray.", true;
    KEY_CONFIG_TUI_RENDER_MARGIN = "config", "tui", "renderMargin", Int, DefaultValue::Int(64), "Rendered blocks kept around the viewport.", true;
    KEY_CONFIG_TUI_TODO_FOLD = "config", "tui", "todoFold", BoolOrInt, DefaultValue::Bool(false), "Fold the task list; true = 8 rows, N = N rows.", true;
    KEY_CONFIG_TUI_USAGE_ALIGN = "config", "tui.usage", "align", Text, DefaultValue::Text("center"), "Where the usage panel sits: center, left or right.", true;
    KEY_CONFIG_TUI_FX_USD_CNY = "config", "tui.fx", "usdCny", Float, DefaultValue::Float(6.0), "1 USD in CNY, for display only.", true;

    PATTERN_MODELS_PROVIDER_BASE_URL = "models", "provider.<name>", "baseUrl", Text, DefaultValue::Null, "Request root; must be http(s).", true;
    PATTERN_MODELS_PROVIDER_API = "models", "provider.<name>", "api", Text, DefaultValue::Null, "Wire protocol name; the legal set comes from req.", true;
    PATTERN_MODELS_PROVIDER_API_KEY = "models", "provider.<name>", "apiKey", Text, DefaultValue::Text(""), "Credential; ${ENV_VAR} is expanded when it is used.", false;
    PATTERN_MODELS_PROVIDER_AUTH_HEADER = "models", "provider.<name>", "authHeader", Bool, DefaultValue::Bool(true), "Send Authorization: Bearer <apiKey>.", true;
    PATTERN_MODELS_PROVIDER_MODELS = "models", "provider.<name>", "models", List, DefaultValue::List(NO_ARGS), "The model ids this provider serves.", true;
    PATTERN_MODELS_MODEL_ID = "models", "model.<address>", "id", Text, DefaultValue::Null, "The wire model id.", true;
    PATTERN_MODELS_MODEL_NAME = "models", "model.<address>", "name", Text, DefaultValue::Null, "Display name; falls back to the id.", true;
    PATTERN_MODELS_MODEL_CONTEXT_WINDOW = "models", "model.<address>", "contextWindow", Int, DefaultValue::Null, "Context window; absent = req decides.", true;
    PATTERN_MODELS_MODEL_MAX_OUTPUT_TOKENS = "models", "model.<address>", "maxOutputTokens", Int, DefaultValue::Null, "max_tokens per request; absent = req decides.", true;
    PATTERN_MODELS_MODEL_CURRENCY = "models", "model.<address>", "currency", Text, DefaultValue::Text("CNY"), "Price display currency: CNY or USD.", true;
    PATTERN_MODELS_MODEL_COST_INPUT = "models", "model.<address>.cost", "input", Float, DefaultValue::Float(0.0), "Input price per 1M tokens.", true;
    PATTERN_MODELS_MODEL_COST_OUTPUT = "models", "model.<address>.cost", "output", Float, DefaultValue::Float(0.0), "Output price per 1M tokens.", true;
    PATTERN_MODELS_MODEL_COST_CACHE_READ = "models", "model.<address>.cost", "cacheRead", Float, DefaultValue::Float(0.0), "Cache-read price per 1M tokens.", true;
    PATTERN_MODELS_MODEL_COST_CACHE_WRITE = "models", "model.<address>.cost", "cacheWrite", Float, DefaultValue::Float(0.0), "Cache-write price per 1M tokens.", true;
}
