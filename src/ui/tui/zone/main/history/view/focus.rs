//! Focus: put a block's head on the row below the sight line.

use super::super::local::section::Idx;
use super::super::prefetch::At;
use super::super::prefetch::walk;
use super::state::{Scene, View};

/// Put this block's head one row below the sight line; `false` = nothing there draws.
pub fn focus(view: &mut View, scene: &mut Scene, idx: Idx) -> bool {
    let mut cursor = Some(idx);
    while let Some(target) = cursor {
        if scene.run.head(target).is_none() {
            scene
                .run
                .lay_from(scene.local, scene.products, scene.opts, target);
        }
        if scene.run.head(target).is_some() {
            if scene.run.head(target) == Some(0) {
                let before = scene.run.len();
                let grew = scene
                    .run
                    .extend_older(scene.local, scene.products, scene.opts)
                    && scene.run.len() > before;
                if !grew {
                    view.set_at(Some(At {
                        idx: target,
                        piece: 0,
                        row: 0,
                    }));
                    return true;
                }
            }
            let head = scene.run.head(target).unwrap_or(0);
            let above = scene.run.at_of(head.saturating_sub(1));
            view.set_at(Some(above.unwrap_or(At {
                idx: target,
                piece: 0,
                row: 0,
            })));
            return true;
        }
        cursor = walk::newer(scene.local, target);
    }
    false
}
