//! Blocks laid out when they enter the sight line.

use super::super::resize::ResizePlan;

pub fn nums(plan: &ResizePlan) -> &[i64] {
    &plan.lazy
}
