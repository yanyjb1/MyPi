use super::provider::SearchProvider;

pub(super) mod bing;
pub(super) mod duckduckgo;
pub(super) mod ecosia;
pub(super) mod google;
pub(super) mod mojeek;
pub(super) mod startpage;

pub(super) const PROVIDERS: &[&dyn SearchProvider] = &[
    &google::Google,
    &bing::Bing,
    &duckduckgo::DuckDuckGo,
    &ecosia::Ecosia,
    &startpage::Startpage,
    &mojeek::Mojeek,
];
