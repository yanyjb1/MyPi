//! The one storage shape the api renames for the wire.

use serde_json::{Value, json};

use crate::server::storage::Cut;

pub(crate) fn cut(value: Cut) -> Value {
    match value {
        Cut::Boundary { turn } => json!({"kind": "boundary", "turn": turn}),
        Cut::Rescued { turn } => json!({"kind": "rescued", "turn": turn}),
        Cut::Retyped { turn } => json!({"kind": "retyped", "turn": turn}),
    }
}
