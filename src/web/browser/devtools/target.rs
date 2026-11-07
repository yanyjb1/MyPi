use serde::Deserialize;

/// One DevTools target: what a tab is keyed by, and the socket to speak CDP on.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Target {
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub ws_url: String,
}
