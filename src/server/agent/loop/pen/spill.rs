//! One artifact row, keyed by the block it spilled from.

use crate::server::agent::r#loop::turn::ask::Artifact;
use crate::server::storage::{Ref, Row};

pub fn rows(block: Ref, artifact: Artifact) -> Vec<Row> {
    vec![Row::Artifact {
        block,
        name: artifact.name,
        content: artifact.content,
        hash: artifact.hash,
        reference: artifact.reference,
    }]
}
