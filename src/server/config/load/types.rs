//! Loading: layers, per-key merge, warnings, bootstrap, and the `Config` handle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_yaml::Mapping;

use super::super::model::ModelsConfig;
use super::super::notice::ConfigNotice;
use super::super::paths::ConfigPaths;
use super::super::registry::Value;
#[cfg(feature = "web")]
use super::super::rules::Rules;

#[derive(Clone)]
pub struct Config {
    pub(super) inner: Arc<Inner>,
}

pub(super) struct Inner {
    pub(super) paths: ConfigPaths,
    pub(super) models: ModelsConfig,
    pub(super) layers: Layers,
    pub(super) cache: Mutex<HashMap<String, Arc<Effective>>>,
    /// One scope per project folder a session has stood in, read on first use.
    pub(super) projects: Mutex<HashMap<PathBuf, Arc<ProjectScope>>>,
    /// The same, for the values a scope resolves: the key carries the scope and the profile.
    pub(super) scoped: Mutex<HashMap<String, Arc<Effective>>>,
    /// The site rules the web tool narrows a page with: read once, beside `config.yaml`.
    #[cfg(feature = "web")]
    pub(super) web_rules: Arc<Rules>,
    pub(super) load_notices: Vec<ConfigNotice>,
    pub(super) runtime: Mutex<Vec<ConfigNotice>>,
}

pub(super) struct Layers {
    pub(super) project: Option<(PathBuf, Mapping)>,
    pub(super) global: Option<(PathBuf, Mapping)>,
}

/// One directory's config roots and its project layer.
pub(super) struct ProjectScope {
    pub(super) paths: ConfigPaths,
    pub(super) layer: Option<(PathBuf, Mapping)>,
}

pub struct Effective {
    pub values: HashMap<&'static str, Value>,
    pub notices: Vec<ConfigNotice>,
}

/// What the web domain needs when it starts or reclaims a browser.
#[cfg(feature = "web")]
#[derive(Debug)]
pub struct BrowserOptions {
    pub bin: Option<std::path::PathBuf>,
    pub port: Option<u16>,
    pub profile_dir: Option<std::path::PathBuf>,
    pub idle: std::time::Duration,
    pub user_agent: Option<String>,
    pub extra_args: Vec<String>,
}
