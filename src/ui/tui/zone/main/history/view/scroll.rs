//! One scroll: the sight line moves, and the run is filled to follow it.

use super::super::prefetch::At;
use super::budget::Budget;
use super::state::{Dir, Scene, View};
use super::window;

/// Move the sight line `count` rows; `false` = it could not move that way.
pub fn step(view: &mut View, scene: &mut Scene, dir: Dir, count: usize) -> bool {
    let height = usize::from(view.rows()).max(1);
    let budget = Budget::of(height as u16, view.focus_line());
    let top = window::top(view, scene, budget);
    let at = view.at().or_else(|| scene.run.at_of(top + budget.above));
    let Some(at) = at else {
        return false;
    };
    match shifted(scene, at, dir, count.max(1)) {
        Some(next) => {
            let index = scene.run.index_of(next);
            let last = scene.run.len().saturating_sub(budget.below);
            if dir == Dir::Newer && index.is_some_and(|index| index >= last) {
                view.follow();
                return true;
            }
            view.set_at(Some(next));
            true
        }
        None if dir == Dir::Newer => {
            view.follow();
            true
        }
        None => false,
    }
}

/// The place `count` rows away, filling the run as needed.
fn shifted(scene: &mut Scene, at: At, dir: Dir, count: usize) -> Option<At> {
    let mut cur = at;
    for _ in 0..count {
        let index = scene.run.index_of(cur)?;
        let next = match dir {
            Dir::Older => {
                if index == 0 {
                    let before = scene.run.len();
                    if !scene
                        .run
                        .extend_older(scene.local, scene.products, scene.opts)
                        || scene.run.len() == before
                    {
                        return None;
                    }
                }
                let index = scene.run.index_of(cur)?;
                index.checked_sub(1)?
            }
            Dir::Newer => {
                let next = index + 1;
                while scene.run.row(next).is_none() {
                    if !scene
                        .run
                        .extend_newer(scene.local, scene.products, scene.opts)
                    {
                        return None;
                    }
                }
                next
            }
        };
        cur = scene.run.at_of(next)?;
    }
    Some(cur)
}
