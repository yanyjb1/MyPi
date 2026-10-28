//! The subagent service: one child per task, and the final draft that child leaves.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};

use crate::server::agent::tools::Env;
use crate::server::agent::tools::jobs::{Id, JobRunner, JobService, JobSpec, Schedule};
use crate::server::agent::tools::spec::{Args, ToolOutput};
use crate::server::storage::{Effect, EffectKind, Writer};

use super::contract;
use super::draft::{self, Drafts};
use super::runner::AgentRunner;
use super::spawn::{self, ChildSpec, Spawn};
use super::template;

pub(crate) struct SubagentFactory {
    max_tool_calls: u32,
    writer: Writer,
    spawn: Arc<dyn Spawn>,
    drafts: Drafts,
    children: Arc<Mutex<HashMap<i64, ChildSpec>>>,
    pending: Arc<Mutex<HashMap<i64, Id>>>,
}

impl SubagentFactory {
    pub(crate) fn new(max_tool_calls: u32, writer: Writer, spawn: Arc<dyn Spawn>) -> Self {
        Self {
            max_tool_calls,
            writer,
            spawn,
            drafts: Drafts::default(),
            children: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn make(
        self: &Arc<Self>,
        session: i64,
        parent_root: PathBuf,
        parent_tools: BTreeSet<String>,
        parent_system: String,
        profile: String,
        model: Option<(String, String)>,
        jobs: Option<Arc<JobService>>,
        child: bool,
    ) -> Arc<Subagent> {
        Arc::new(Subagent {
            factory: self.clone(),
            session,
            parent_root,
            parent_tools,
            parent_system,
            profile,
            model,
            jobs,
            child,
        })
    }

    pub(crate) fn child_spec(&self, child: i64) -> Option<ChildSpec> {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&child)
            .cloned()
    }

    pub(crate) fn drafts(&self) -> Drafts {
        self.drafts.clone()
    }
}

pub(crate) struct Subagent {
    factory: Arc<SubagentFactory>,
    session: i64,
    parent_root: PathBuf,
    parent_tools: BTreeSet<String>,
    parent_system: String,
    profile: String,
    model: Option<(String, String)>,
    jobs: Option<Arc<JobService>>,
    child: bool,
}

pub(crate) struct TaskReceipt {
    pub(crate) child: i64,
    pub(crate) task: Id,
    pub(crate) final_path: PathBuf,
}

impl TaskReceipt {
    pub(crate) fn text(&self) -> String {
        format!(
            "subsession {}, task {}, final {}",
            self.child,
            self.task,
            self.final_path.display()
        )
    }
}

impl Subagent {
    pub(crate) fn is_child(&self) -> bool {
        self.child
    }

    pub(crate) fn is_parent(&self) -> bool {
        !self.child
    }

    pub(crate) fn task(
        &self,
        env: &Env,
        template_name: &str,
        vars: BTreeMap<String, String>,
        cwd: Option<&str>,
        max_tool_calls: Option<u32>,
    ) -> Result<TaskReceipt> {
        anyhow::ensure!(!self.child, "a child cannot spawn another subagent");
        let jobs = self
            .jobs
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no job service for this task"))?;
        let template = template::load(env, template_name, &vars, &self.parent_tools)?;
        let cwd = match cwd {
            Some(raw) => {
                anyhow::ensure!(
                    self.parent_tools.contains("cd"),
                    "`cwd` needs the parent's `cd` tool"
                );
                let path = if Path::new(raw).is_absolute() {
                    PathBuf::from(raw)
                } else {
                    env.workspace.cwd().join(raw)
                };
                let path = path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve cwd `{raw}`"))?;
                anyhow::ensure!(path.is_dir(), "cwd `{raw}` is not a directory");
                path
            }
            None => env.workspace.cwd().to_path_buf(),
        };
        let max = max_tool_calls.unwrap_or(self.factory.max_tool_calls);
        anyhow::ensure!(max > 0, "`maxToolCalls` must be positive");
        let system = format!(
            "{}\n\n{}\n\n{}",
            self.parent_system,
            contract::SYSTEM,
            template.body
        );
        let child = spawn::create(
            &self.factory.writer,
            cwd.to_string_lossy().as_ref(),
            self.model.clone(),
            &self.profile,
        )?;
        let spec = ChildSpec {
            parent: self.session,
            parent_root: self.parent_root.clone(),
            system,
            tools: template.tools,
            max_tool_calls: max,
        };
        self.factory
            .children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(child, spec);
        let runner: Arc<dyn JobRunner> = Arc::new(AgentRunner::new(
            child,
            self.parent_root.clone(),
            self.factory.spawn.clone(),
            self.factory.drafts.clone(),
        ));
        let job = match jobs.reserve(JobSpec {
            runner,
            schedule: Schedule::Now,
            timeout: None,
            output: false,
            needs_slot: true,
            kind: "agent",
            wake_on_done: true,
        }) {
            Ok(job) => job,
            Err(e) => {
                self.factory
                    .children
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&child);
                let _ = self.factory.writer.delete_session(child);
                return Err(e);
            }
        };
        self.factory
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(child, job.id);
        Ok(TaskReceipt {
            child,
            task: job.id,
            final_path: draft::path(&self.parent_root, child),
        })
    }

    pub(crate) fn activate_effects(&self, effects: &[Effect]) {
        let Some(jobs) = &self.jobs else {
            return;
        };
        for effect in effects {
            let EffectKind::Subsession { session } = &effect.kind else {
                continue;
            };
            let id = self
                .factory
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(session);
            if let Some(id) = id
                && let Err(e) = jobs.activate(id)
            {
                crate::server::platform::log::warn(
                    "agent",
                    format!("subsession {session}: task {id} did not activate: {e:#}"),
                );
            }
        }
    }

    pub(crate) fn final_op(&self, args: &Args) -> Result<ToolOutput> {
        anyhow::ensure!(self.child, "`final` is only available to a subagent");
        let op = args.choice(0)?;
        let text = match op.as_str() {
            "write" => {
                self.factory.drafts.write(self.session, args.str(1)?);
                "draft written".to_string()
            }
            "edit" => self
                .factory
                .drafts
                .edit(self.session, &args.str(2)?, &args.str(3)?)?,
            "mass_edit" => {
                let rows = args.table(4)?;
                let mut edits = Vec::with_capacity(rows.len());
                for row in &rows {
                    edits.push((row.line(0)?, row.str(1)?));
                }
                self.factory.drafts.mass_edit(self.session, &edits)?
            }
            "read" => self.factory.drafts.read(self.session),
            "submit" => {
                let path = self
                    .factory
                    .drafts
                    .submit(&self.parent_root, self.session)?;
                return Ok(ToolOutput::text(format!("submitted {}", path.display())).stop());
            }
            _ => unreachable!(),
        };
        Ok(ToolOutput::text(text))
    }
}
