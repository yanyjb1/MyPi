//! Lay out the blocks the sight line can see right now.

use super::super::cache::{BlockCache, Item};
use super::super::render::theme::HistoryTheme;

pub fn apply(cache: &mut BlockCache, items: &[Item<'_>], nums: &[i64], t: &HistoryTheme) {
    for item in items {
        if nums.contains(&item.num) {
            cache.ensure_item(item, t);
        }
    }
}
