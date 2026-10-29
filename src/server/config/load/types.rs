//! Loading: layers, per-key merge, warnings, bootstrap, and the `Config` handle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_yaml::Mapping;

use super::super::model::ModelsConfig;
use super::super::notice::ConfigNotice;
use super::super::paths::ConfigPaths;
use super::super::registry::Value;

#[derive(Clone)]
pub struct Config {
    pub(super) inner: Arc<Inner>,
}

pub(super) struct Inner {
    pub(super) paths: ConfigPaths,
    pub(super) models: ModelsConfig,
    pub(super) layers: Layers,
    pub(super) cache: Mutex<HashMap<String, Arc<Effective>>>,
    pub(super) load_notices: Vec<ConfigNotice>,
    pub(super) runtime: Mutex<Vec<ConfigNotice>>,
}

pub(super) struct Layers {
    pub(super) project: Option<(PathBuf, Mapping)>,
    pub(super) global: Option<(PathBuf, Mapping)>,
}

pub struct Effective {
    pub values: HashMap<&'static str, Value>,
    pub notices: Vec<ConfigNotice>,
}
