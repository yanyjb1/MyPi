//! A generic read plan: a sheet, some columns, a conjunction of conditions. The reader turns it
//! into SQL; callers bind names and values and never write SQL.

/// One table that may be read this way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    Blocks,
    Memos,
    Aliases,
    Sessions,
    Turns,
    Cwds,
}

impl Sheet {
    pub fn name(self) -> &'static str {
        match self {
            Sheet::Blocks => "blocks",
            Sheet::Memos => "memos",
            Sheet::Aliases => "aliases",
            Sheet::Sessions => "sessions",
            Sheet::Turns => "turns",
            Sheet::Cwds => "cwd_history",
        }
    }

    /// Logical name → column. The logical side is what a row's JSON carries.
    pub fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Sheet::Blocks => &[
                ("id", "block_id"),
                ("kind", "type"),
                ("content", "content"),
                ("prefix", "prefix"),
                ("suffix", "suffix"),
                ("usage_total", "usage_total"),
                ("usage_prompt", "usage_prompt"),
                ("usage_completion", "usage_completion"),
                ("usage_cached", "usage_cached"),
                ("usage_cache_write", "usage_cache_write"),
                ("usage_reasoning", "usage_reasoning"),
                ("call_id", "call_id"),
                ("name", "name"),
                ("args", "args"),
                ("intent", "intent"),
                ("text", "text"),
                ("first", "first"),
                ("ok", "ok"),
                ("result", "result"),
                ("memo", "memo"),
                ("duration_ms", "duration_ms"),
                ("ts", "ts"),
            ],
            Sheet::Memos => &[
                ("id", "id"),
                ("anchor", "anchor"),
                ("name", "name"),
                ("content", "content"),
                ("ts", "ts"),
            ],
            Sheet::Aliases => &[
                ("id", "id"),
                ("anchor", "anchor"),
                ("name", "name"),
                ("command", "command"),
                ("ts", "ts"),
            ],
            Sheet::Sessions => &[
                ("id", "id"),
                ("name", "name"),
                ("started_at", "started_at"),
                ("cwd", "cwd"),
                ("provider", "provider"),
                ("model", "model"),
                ("anchor", "anchor"),
                ("meta", "meta"),
            ],
            Sheet::Turns => &[
                ("turn_id", "turn_id"),
                ("ranges", "blocks"),
                ("stop_reason", "stop_reason"),
            ],
            Sheet::Cwds => &[
                ("session_id", "session_id"),
                ("turn_id", "turn_id"),
                ("cwd", "cwd"),
            ],
        }
    }

    /// The SQL column a logical name stands for.
    pub fn column(self, logical: &str) -> Option<&'static str> {
        self.columns()
            .iter()
            .find(|(name, _)| *name == logical)
            .map(|(_, sql)| *sql)
    }
}

/// One value a condition carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Int(i64),
    Text(String),
}

/// One condition. All conditions are ANDed.
#[derive(Debug, Clone)]
pub enum Cond {
    Eq(&'static str, Val),
    In(&'static str, Vec<Val>),
    /// `LIKE '%text%'`.
    Contains(&'static str, String),
    /// Membership in any of the intervals.
    Within(&'static str, Vec<[i64; 2]>),
    Null(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Asc,
    Desc,
}

/// One read.
#[derive(Debug, Clone)]
pub struct Query {
    pub sheet: Sheet,
    /// Logical column names; empty means every column of the sheet.
    pub columns: Vec<&'static str>,
    pub filter: Vec<Cond>,
    pub order: Option<(&'static str, Order)>,
    pub limit: Option<usize>,
}

impl Query {
    pub fn new(sheet: Sheet) -> Self {
        Self {
            sheet,
            columns: Vec::new(),
            filter: Vec::new(),
            order: None,
            limit: None,
        }
    }

    pub fn columns(mut self, columns: &[&'static str]) -> Self {
        self.columns = columns.to_vec();
        self
    }

    pub fn filter(mut self, filter: Vec<Cond>) -> Self {
        self.filter = filter;
        self
    }

    pub fn order(mut self, column: &'static str, order: Order) -> Self {
        self.order = Some((column, order));
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}
