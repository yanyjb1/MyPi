use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;

use super::budget::ToolBudget;
use super::outlet::{Outlet, ToolEvent};
use crate::server::agent::request::ToolRun;
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::tools::{self, Env, ToolProgress, Workspace};
use crate::server::storage::{Relay, SessionChange, Submit, Writer};

/// Per-session tool state: cwd and budget.
pub struct Driver {
    pub workspace: Workspace,
    pub budget: ToolBudget,
    session: i64,
}

impl Driver {
    pub fn new(session: i64, workspace: Workspace, budget: ToolBudget) -> Self {
        Self {
            workspace,
            budget,
            session,
        }
    }

    pub fn cwd(&self) -> &Path {
        self.workspace.cwd()
    }

    pub fn snapshot(&self) -> PathBuf {
        self.cwd().to_path_buf()
    }

    pub fn set_cwd(&mut self, w: &Writer, turn: Option<i64>, path: &str) -> Result<()> {
        w.push(Submit::Session {
            session: self.session,
            change: SessionChange::Cwd {
                turn,
                path: path.to_string(),
            },
        })?;
        self.workspace.chdir(PathBuf::from(path));
        Ok(())
    }

    pub fn run_calls(
        &mut self,
        w: &Writer,
        env: &mut Env,
        reply: &AssistantMessage,
        out: &mut dyn Outlet,
        turn: Option<i64>,
    ) -> Result<Vec<ToolRun>> {
        let snapshot = self.snapshot();
        env.workspace.chdir(snapshot.clone());
        let mut ran = Vec::with_capacity(reply.tool_calls.len());

        for (i, call) in reply.tool_calls.iter().enumerate() {
            out.tool_event(&ToolEvent::Start {
                call_id: call.id.clone(),
                name: call.name().to_string(),
                args: call.function.arguments.clone(),
                intent: call.intent(),
                text: if i == 0 {
                    reply.content.clone()
                } else {
                    String::new()
                },
                first: i == 0,
            });

            let started = Instant::now();
            if call.name() == "final" && final_submit(call) && reply.tool_calls.len() > 1 {
                let result = "`final submit` must be the only call in its message".to_string();
                let duration_ms = started.elapsed().as_millis() as u64;
                out.tool_event(&ToolEvent::Finish {
                    call_id: call.id.clone(),
                    name: call.name().to_string(),
                    ok: false,
                    result: result.clone(),
                    memo: None,
                    duration_ms,
                });
                ran.push(ToolRun {
                    call_id: call.id.clone(),
                    ok: false,
                    result,
                    memo: None,
                    spill: None,
                    duration_ms,
                    effects: Vec::new(),
                    stop: false,
                });
                continue;
            }
            if !self.budget.allows(call.name()) {
                let result = "budget exhausted; only `final` is allowed".to_string();
                let duration_ms = started.elapsed().as_millis() as u64;
                out.tool_event(&ToolEvent::Finish {
                    call_id: call.id.clone(),
                    name: call.name().to_string(),
                    ok: false,
                    result: result.clone(),
                    memo: None,
                    duration_ms,
                });
                ran.push(ToolRun {
                    call_id: call.id.clone(),
                    ok: false,
                    result,
                    memo: None,
                    spill: None,
                    duration_ms,
                    effects: Vec::new(),
                    stop: false,
                });
                continue;
            }

            self.budget.record(call.name());
            let call_id = call.id.clone();
            let outcome = {
                let session = self.session;
                let mut progress = |p: ToolProgress| {
                    if let ToolProgress::Output(text) = p
                        && let Some(turn) = turn
                    {
                        let _ = w.stream(Relay::Tool {
                            session,
                            turn,
                            call_id: call_id.clone(),
                            text,
                        });
                    }
                };
                tools::execute(env, call, &mut progress)
            };
            let (ok, result, memo, spill, effects, stop) = match outcome {
                Ok(out) => (true, out.text, out.memo, out.spill, out.effects, out.stop),
                Err(e) => (
                    false,
                    format!("tool execution failed: {e:#}"),
                    None,
                    None,
                    Vec::new(),
                    false,
                ),
            };
            let duration_ms = started.elapsed().as_millis() as u64;

            if ok && call.name() == "cd" {
                let path = env.workspace.cwd().to_string_lossy().into_owned();
                self.set_cwd(w, turn, &path)?;
                env.workspace.chdir(snapshot.clone());
            }

            out.tool_event(&ToolEvent::Finish {
                call_id: call.id.clone(),
                name: call.name().to_string(),
                ok,
                result: result.clone(),
                memo: memo.clone(),
                duration_ms,
            });
            ran.push(ToolRun {
                call_id: call.id.clone(),
                ok,
                result,
                memo,
                spill,
                duration_ms,
                effects,
                stop,
            });
        }
        Ok(ran)
    }
}

fn final_submit(call: &crate::server::agent::request::message::ToolCall) -> bool {
    serde_json::from_str::<serde_json::Value>(&call.function.arguments)
        .ok()
        .and_then(|v| {
            v.get("op")
                .and_then(|v| v.as_str())
                .map(|op| op == "submit")
        })
        .unwrap_or(false)
}
