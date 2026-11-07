use super::provider::SearchProvider;

pub(super) mod baidu;
pub(super) mod bing;
pub(super) mod google;

pub(super) const PROVIDERS: &[&dyn SearchProvider] = &[&google::Google, &bing::Bing, &baidu::Baidu];
