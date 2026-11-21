// `blocks` — closed id ranges, answered in request order.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64, ranges: Vec<[i64; 2]>) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let owned = reader.ranges(session)?;
    let mut found = Vec::new();
    for [lo, hi] in ranges {
        if lo > hi {
            return Err(Refuse::new("range_reversed").args(json!({"lo": lo, "hi": hi})));
        }
        if !owned.iter().any(|[start, end]| *start <= lo && hi <= *end) {
            return Err(
                Refuse::new("range_outside").args(json!({"lo": lo, "hi": hi, "session": session}))
            );
        }
        found.extend(reader.blocks(&[[lo, hi]])?);
    }
    Ok(json!({"session": session, "blocks": found}))
}
