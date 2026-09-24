//! Search providers: one file per engine/site. A provider turns a query
//! into `Vec<SearchHit>` and knows nothing about routing or fallback —
//! that is `engine.rs`'s job.

pub(super) mod bing;
pub(super) mod ddg;

pub(super) use bing::direct;
pub(super) use ddg::via_browser;
