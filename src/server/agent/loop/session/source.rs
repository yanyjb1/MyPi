//! The loop's read ports: what the tools may know about a session, and nothing more.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use crate::server::agent::tools::contract::{
    BlockPair, CallRecord, Child, Direction, Furnish, MemoRead, MemoReadOp, MemoScope, MemoVersion,
    MemoView, Source,
};
use crate::server::agent::tools::{JobService, McpBridge};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet, Val};

/// One database path, plus the handles the loop attaches as sessions come alive.
pub struct Db {
    path: PathBuf,
    jobs: Mutex<HashMap<i64, Arc<JobService>>>,
    children: Mutex<HashMap<i64, Arc<dyn Child>>>,
    mcp: Mutex<Option<Arc<dyn McpBridge>>>,
}

impl Db {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            jobs: Mutex::new(HashMap::new()),
            children: Mutex::new(HashMap::new()),
            mcp: Mutex::new(None),
        })
    }

    fn reader(&self) -> Result<Reader> {
        Reader::open(&self.path)
    }

    pub fn attach_jobs(&self, session: i64, service: Arc<JobService>) {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session, service);
    }

    pub fn attach_child(&self, session: i64, child: Arc<dyn Child>) {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session, child);
    }

    pub fn attach_mcp(&self, bridge: Arc<dyn McpBridge>) {
        *self.mcp.lock().unwrap_or_else(|e| e.into_inner()) = Some(bridge);
    }
}

impl Furnish for Db {
    fn mcp(&self) -> Option<Arc<dyn McpBridge>> {
        self.mcp.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Source for Db {
    fn memo(&self, _session: i64) -> Arc<dyn MemoRead> {
        Arc::new(Memos {
            path: self.path.clone(),
        })
    }

    fn aliases(&self, session: i64) -> Result<BTreeMap<String, String>> {
        let reader = self.reader()?;
        let ranges = ranges(&reader, session)?;
        let rows = reader.query(
            &Query::new(Sheet::Aliases)
                .columns(&["id", "name", "command"])
                .filter(vec![Cond::Within("anchor", ranges)])
                .order("id", Order::Asc),
        )?;
        let mut out = BTreeMap::new();
        for row in rows {
            if let (Some(name), Some(command)) = (row["name"].as_str(), row["command"].as_str()) {
                out.insert(name.to_string(), command.to_string());
            }
        }
        Ok(out)
    }

    fn history(
        &self,
        session: i64,
        anchor: &str,
        direction: Direction,
        limit: usize,
        tool: Option<&str>,
    ) -> Result<Vec<CallRecord>> {
        let reader = self.reader()?;
        let ranges = ranges(&reader, session)?;
        let anchor = anchor_block(&reader, anchor)?;
        let ranges = match direction {
            Direction::Back => clip(ranges, ..anchor),
            Direction::Forward => clip(ranges, anchor + 1..),
        };
        let mut filter = vec![
            Cond::Eq("kind", Val::Text("tool_request".to_string())),
            Cond::Within("id", ranges),
        ];
        if let Some(tool) = tool
            && tool != "all"
        {
            filter.push(Cond::Eq("name", Val::Text(tool.to_string())));
        }
        let order = match direction {
            Direction::Back => Order::Desc,
            Direction::Forward => Order::Asc,
        };
        let requests = reader.query(
            &Query::new(Sheet::Blocks)
                .columns(&["id", "kind", "name", "call_id", "args"])
                .filter(filter)
                .order("id", order)
                .limit(limit),
        )?;
        Ok(records(&reader, requests)?)
    }

    fn calls_after(&self, session: i64, block: i64, tool: &str) -> Result<Vec<CallRecord>> {
        let reader = self.reader()?;
        let ranges = clip(ranges(&reader, session)?, block + 1..);
        let requests = reader.query(
            &Query::new(Sheet::Blocks)
                .columns(&["id", "kind", "name", "call_id", "args"])
                .filter(vec![
                    Cond::Eq("kind", Val::Text("tool_request".to_string())),
                    Cond::Eq("name", Val::Text(tool.to_string())),
                    Cond::Within("id", ranges),
                ])
                .order("id", Order::Asc),
        )?;
        Ok(records(&reader, requests)?)
    }

    fn todo_on(&self, session: i64) -> Result<bool> {
        let reader = self.reader()?;
        let rows = reader.query(
            &Query::new(Sheet::Sessions)
                .columns(&["todo_on"])
                .filter(vec![Cond::Eq("id", Val::Int(session))])
                .limit(1),
        )?;
        Ok(rows
            .first()
            .and_then(|row| row["todo_on"].as_i64())
            .unwrap_or(0)
            != 0)
    }

    fn todo_off(&self, session: i64) -> Result<Option<i64>> {
        let reader = self.reader()?;
        let ranges = ranges(&reader, session)?;
        let rows = reader.query(
            &Query::new(Sheet::Blocks)
                .columns(&["id"])
                .filter(vec![
                    Cond::Eq("kind", Val::Text("tool_result".to_string())),
                    Cond::Contains("memo", "\"todo_on\":false".to_string()),
                    Cond::Within("id", ranges),
                ])
                .order("id", Order::Desc)
                .limit(1),
        )?;
        Ok(rows.first().and_then(|row| row["id"].as_i64()))
    }

    fn block(&self, session: i64, id: i64) -> Result<Option<BlockPair>> {
        let reader = self.reader()?;
        let ranges = ranges(&reader, session)?;
        let rows = reader.query(&Query::new(Sheet::Blocks).filter(vec![
            Cond::Eq("id", Val::Int(id)),
            Cond::Within("id", ranges),
        ]))?;
        let Some(request) = rows.into_iter().next() else {
            return Ok(None);
        };
        if request["kind"] != "tool_request" {
            return Ok(None);
        }
        let Some(call_id) = request["call_id"].as_str() else {
            return Ok(None);
        };
        let result = reader
            .query(
                &Query::new(Sheet::Blocks)
                    .filter(vec![
                        Cond::Eq("kind", Val::Text("tool_result".to_string())),
                        Cond::Eq("call_id", Val::Text(call_id.to_string())),
                    ])
                    .limit(1),
            )?
            .into_iter()
            .next();
        Ok(Some(BlockPair { request, result }))
    }

    fn jobs(&self, session: i64) -> Option<Arc<JobService>> {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session)
            .cloned()
    }

    fn child(&self, session: i64) -> Option<Arc<dyn Child>> {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session)
            .cloned()
    }
}

/// The memo reads, scoped by the session's visible blocks.
struct Memos {
    path: PathBuf,
}

impl MemoRead for Memos {
    fn read(
        &self,
        session: i64,
        scope: MemoScope,
        op: MemoReadOp,
        name: Option<&str>,
    ) -> Result<MemoView> {
        let reader = Reader::open(&self.path)?;
        let mut filter = match scope {
            MemoScope::Current => vec![Cond::Within("anchor", ranges(&reader, session)?)],
            MemoScope::Global => vec![Cond::Null("anchor")],
        };
        if let Some(name) = name {
            filter.push(Cond::Eq("name", Val::Text(name.to_string())));
        }
        let rows = reader.query(
            &Query::new(Sheet::Memos)
                .columns(&["id", "name", "content", "ts"])
                .filter(filter)
                .order("id", Order::Asc),
        )?;
        Ok(match op {
            MemoReadOp::List => {
                let mut names = BTreeSet::new();
                for row in &rows {
                    if let Some(name) = row["name"].as_str() {
                        names.insert(name.to_string());
                    }
                }
                MemoView::Names(names.into_iter().collect())
            }
            MemoReadOp::Show => {
                let name = name.ok_or_else(|| anyhow!("a memo name is required"))?;
                let found = rows
                    .last()
                    .and_then(|row| row["content"].as_str())
                    .ok_or_else(|| anyhow!("no memo named {name}"))?;
                MemoView::Content(found.to_string())
            }
            MemoReadOp::History => {
                let name = name.ok_or_else(|| anyhow!("a memo name is required"))?;
                let mut out = Vec::new();
                for row in &rows {
                    let (Some(id), Some(content), Some(ts)) = (
                        row["id"].as_i64(),
                        row["content"].as_str(),
                        row["ts"].as_str(),
                    ) else {
                        continue;
                    };
                    out.push(MemoVersion {
                        id,
                        content: content.to_string(),
                        ts: ts.to_string(),
                    });
                }
                if out.is_empty() {
                    return Err(anyhow!("no memo named {name}"));
                }
                MemoView::History(out)
            }
        })
    }
}

/// Every block range the session's list covers.
pub(crate) fn ranges(reader: &Reader, session: i64) -> Result<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for row in reader.turns(session)? {
        let Some(items) = row["ranges"].as_array() else {
            continue;
        };
        for item in items {
            let Some(pair) = item.as_array().filter(|pair| pair.len() == 2) else {
                bail!("a turn's range is not a pair");
            };
            out.push([pair[0].as_i64().unwrap_or(0), pair[1].as_i64().unwrap_or(0)]);
        }
    }
    Ok(out)
}

/// Keep only the part of every range inside `bounds`.
pub(crate) fn clip(
    ranges: Vec<[i64; 2]>,
    bounds: impl std::ops::RangeBounds<i64>,
) -> Vec<[i64; 2]> {
    use std::ops::Bound;
    let lo = match bounds.start_bound() {
        Bound::Included(n) => *n,
        Bound::Excluded(n) => n + 1,
        Bound::Unbounded => i64::MIN,
    };
    let hi = match bounds.end_bound() {
        Bound::Included(n) => *n,
        Bound::Excluded(n) => n - 1,
        Bound::Unbounded => i64::MAX,
    };
    ranges
        .into_iter()
        .filter_map(|[start, end]| {
            let start = start.max(lo);
            let end = end.min(hi);
            (start <= end).then_some([start, end])
        })
        .collect()
}

/// The block a `#ref` or a number names.
fn anchor_block(reader: &Reader, anchor: &str) -> Result<i64> {
    let text = anchor.trim();
    if let Some(reference) = text.strip_prefix('#') {
        let rows = reader.artifacts_by_ref(reference)?;
        return rows
            .first()
            .and_then(|row| row["block_id"].as_i64())
            .ok_or_else(|| anyhow!("unknown artifact #{reference}"));
    }
    text.parse::<i64>()
        .map_err(|_| anyhow!("an anchor is `#<ref>` or a block id"))
}

/// Request rows plus their paired results, as history hands them over.
fn records(reader: &Reader, requests: Vec<Value>) -> Result<Vec<CallRecord>> {
    let ids: Vec<Val> = requests
        .iter()
        .filter_map(|row| row["call_id"].as_str())
        .map(|call| Val::Text(call.to_string()))
        .collect();
    let mut paired: HashMap<String, (Option<String>, bool)> = HashMap::new();
    if !ids.is_empty() {
        let results = reader.query(
            &Query::new(Sheet::Blocks)
                .columns(&["call_id", "result", "ok"])
                .filter(vec![
                    Cond::Eq("kind", Val::Text("tool_result".to_string())),
                    Cond::In("call_id", ids),
                ]),
        )?;
        for row in results {
            let (Some(call), result, ok) = (
                row["call_id"].as_str(),
                row["result"].as_str().map(str::to_string),
                row["ok"].as_i64().unwrap_or(0) != 0,
            ) else {
                continue;
            };
            paired.insert(call.to_string(), (result, ok));
        }
    }
    Ok(requests
        .into_iter()
        .map(|row| {
            let call = row["call_id"].as_str().unwrap_or_default();
            let (result, ok) = paired
                .get(call)
                .map(|(result, ok)| (result.clone(), *ok))
                .unwrap_or((None, false));
            CallRecord {
                block: row["id"].as_i64(),
                tool: row["name"].as_str().unwrap_or_default().to_string(),
                args: row["args"].as_str().unwrap_or_default().to_string(),
                result,
                ok,
            }
        })
        .collect())
}
