#[derive(Debug, Clone)]
pub struct WebArgs {
    pub op: String,
    pub url: Option<String>,
    pub raw: bool,
    pub query: Option<String>,
    pub provider: Option<String>,
    pub page: u32,
    pub selector: Option<String>,
    pub value: Option<serde_json::Value>,
    pub filter: Option<String>,
    pub max: Option<usize>,
    pub full_page: bool,
    pub reserve: bool,
}
