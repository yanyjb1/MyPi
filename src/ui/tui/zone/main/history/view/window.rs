//! The window: which run rows the screen shows.

use ratatui::text::Line;

use super::super::row::Row;
use super::budget::Budget;
use super::state::{Scene, View};
use crate::ui::tui::theme::Accent;

/// The run index the window starts at, after the run is filled as far as it goes.
pub fn top(view: &mut View, scene: &mut Scene, budget: Budget) -> usize {
    let body = budget.height();
    scene.run.set_width(scene.width, scene.products);
    let Some(at) = view.at() else {
        while scene
            .run
            .extend_newer(scene.local, scene.products, scene.opts)
        {}
        while scene.run.len() < body
            && scene
                .run
                .extend_older(scene.local, scene.products, scene.opts)
        {}
        return scene.run.len().saturating_sub(budget.below);
    };
    if scene.run.index_of(at).is_none() && scene.run.head(at.idx).is_none() {
        scene
            .run
            .lay_from(scene.local, scene.products, scene.opts, at.idx);
    }
    match settled(scene, budget, at) {
        Some(top) => top,
        None => {
            view.follow();
            while scene
                .run
                .extend_newer(scene.local, scene.products, scene.opts)
            {}
            scene.run.len().saturating_sub(budget.below)
        }
    }
}

/// Where the window starts for a sight line that names a row.
fn settled(scene: &mut Scene, budget: Budget, at: super::super::prefetch::At) -> Option<usize> {
    let body = budget.height();
    loop {
        let index = scene.run.index_of(at)?;
        if index >= budget.above {
            let mut top = index - budget.above;
            while scene.run.len() < top + body
                && scene
                    .run
                    .extend_newer(scene.local, scene.products, scene.opts)
            {}
            let last = scene.run.len().saturating_sub(body);
            if top > last {
                top = last;
            }
            return Some(top);
        }
        let before = scene.run.len();
        if !scene
            .run
            .extend_older(scene.local, scene.products, scene.opts)
            || scene.run.len() == before
        {
            return Some(0);
        }
    }
}

/// The screen's rows: the window, bottom-aligned when the run is short, then the pinned rows.
pub fn rows(
    view: &mut View,
    scene: &mut Scene,
    pinned: &[Row],
    accent: Accent,
) -> Vec<Line<'static>> {
    let height = usize::from(view.rows());
    let body = height.saturating_sub(pinned.len()).max(1);
    let budget = Budget::of(body as u16, view.focus_line());
    let top = top(view, scene, budget);
    let len = scene.run.len();
    let shown = len.saturating_sub(top).min(body);
    let mut out: Vec<Line<'static>> = Vec::with_capacity(height);
    for _ in 0..body.saturating_sub(shown) {
        out.push(Line::default());
    }
    for index in top..top + shown {
        match scene.run.row(index) {
            Some(row) => out.push(row.paint(accent)),
            None => out.push(Line::default()),
        }
    }
    out.extend(pinned.iter().map(|row| row.paint(accent)));
    if scene.run.len() > 3 * body {
        scene.run.trim(top.saturating_sub(body), top + 2 * body);
    }
    out
}
