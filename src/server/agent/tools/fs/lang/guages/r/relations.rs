// The R relations: a lone function name and where it is declared.

use std::path::Path;

use super::table::{named, shape};
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{Line, Relation, Relations, Row, Source};

// How R's relations read: one name, declared at the file's own level.
pub(crate) const RELATIONS: Relations = Relations { scan };

// Members one source lists.
const MEMBERS: usize = 12;

// The rows a query answers: where a name is declared, or nothing at all.
fn scan(sources: &[Source], root: &Path, query: &str) -> Relation {
    let Some(name) = ident(query) else {
        return Relation {
            rows: Vec::new(),
            used: Vec::new(),
        };
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (at, src) in sources.iter().enumerate() {
        let mine: Vec<(usize, &Line)> = src
            .table
            .pieces()
            .into_iter()
            .filter(|(_, line)| declares(line, name))
            .collect();
        if mine.is_empty() {
            continue;
        }
        used.push(at);
        let label = parts::rel(root, src.path);
        for (line_at, line) in mine.iter().take(MEMBERS) {
            let end = src.table.end(*line_at).unwrap_or(*line_at);
            rows.push(Row {
                heading: None,
                text: format!(
                    "  {label}:{}  {}  ({}..{})",
                    line_at + 1,
                    line.exact.as_deref().unwrap_or(""),
                    line_at + 1,
                    end + 1
                ),
            });
        }
        if mine.len() > MEMBERS {
            rows.push(Row {
                heading: None,
                text: format!("  [+{} more]", mine.len() - MEMBERS),
            });
        }
    }
    if rows.is_empty() {
        return Relation {
            rows: Vec::new(),
            used: Vec::new(),
        };
    }
    let mut out: Vec<Row> = vec![Row {
        heading: Some(format!("[x-ray] {name}")),
        text: String::new(),
    }];
    out.extend(rows);
    Relation { rows: out, used }
}

// Whether this piece is the definition of `word`, and a definition of some kind at all.
fn declares(line: &Line, word: &str) -> bool {
    matches!(shape(line), Some("function" | "class")) && named(line) == Some(word)
}

// A lone name, in R's own spelling: letters, digits, dots and underscores, either bare or in backticks. A colon belongs to no name of R's.
fn ident(query: &str) -> Option<&str> {
    let query = query.trim();
    let bare = query
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'));
    let bare = bare.unwrap_or(query);
    let mut chars = bare.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '.' || first == '_') {
        return None;
    }
    chars
        .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_'))
        .then_some(bare)
}
