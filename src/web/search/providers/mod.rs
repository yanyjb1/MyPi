use super::provider::SearchProvider;

pub(super) mod baidu;
pub(super) mod bing;
pub(super) mod google;

/// The engine names, read by the schema, the error text and the table. One entry here and one in
/// `PROVIDERS`, same order: a new engine is those two lines plus its module.
pub(super) const NAMES: &[&str] = &[google::NAME, bing::NAME, baidu::NAME];

pub(super) const PROVIDERS: &[&dyn SearchProvider] = &[&google::Google, &bing::Bing, &baidu::Baidu];
