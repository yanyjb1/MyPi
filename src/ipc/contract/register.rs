//! A front end's own id, taken from `hello`.

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Viewer(pub String);
