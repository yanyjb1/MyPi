//! A command frozen at tool-call time; artifacts resolved only when it runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};

use crate::server::agent::tools::Env;
use crate::server::agent::tools::bash::bash::shell::OutputSink;
use crate::server::agent::tools::bash::bash::{expand_alias, guard, rewrite_refs, shell};
use crate::server::agent::tools::jobs::{JobCtx, JobOutcome, JobRunner, OutputFile};
use crate::server::agent::tools::spec::Artifacts;

#[derive(Debug, Clone)]
pub(crate) struct Command {
    cwd: PathBuf,
    zone: PathBuf,
    template: String,
}

impl Command {
    pub(crate) fn prepare(env: &Env, content: &str) -> Result<Self> {
        let text = content.trim();
        anyhow::ensure!(!text.is_empty(), "empty command");
        anyhow::ensure!(
            !text.contains('\n') && !text.contains('\r'),
            "one command at a time"
        );
        let template = expand_alias(text, &env.aliases);
        let cwd = env.workspace.cwd().to_path_buf();
        let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
        if let guard::Verdict::Block(hits) = guard::classify(&template, &zone) {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            bail!("denied: {}", ids.join("+"));
        }
        Ok(Self {
            cwd,
            zone,
            template,
        })
    }

    pub(crate) fn run(
        &self,
        artifacts: &dyn Artifacts,
        timeout: std::time::Duration,
        cancel: Arc<AtomicBool>,
        output: &mut OutputFile,
    ) -> Result<shell::Exit> {
        let cmd = rewrite_refs(&self.template, artifacts)?;
        match guard::classify(&cmd, &self.zone) {
            guard::Verdict::Block(hits) => {
                let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
                bail!("denied: {}", ids.join("+"));
            }
            guard::Verdict::Warn(hits) => {
                let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
                output.out(&format!("[warn: {}]\n", ids.join("+")))?;
            }
            guard::Verdict::Allow => {}
        }
        let mut progress = |_| {};
        shell::run_to(
            &self.cwd,
            &cmd,
            timeout,
            Some(cancel),
            &mut progress,
            output,
        )
    }
}

pub(crate) struct CommandRunner {
    command: Command,
    artifacts: Option<Arc<dyn Artifacts>>,
}

impl CommandRunner {
    pub(crate) fn new(command: Command, artifacts: Option<Arc<dyn Artifacts>>) -> Self {
        Self { command, artifacts }
    }
}

impl JobRunner for CommandRunner {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        let Some(output) = ctx.output.as_mut() else {
            bail!("a command job needs an output file");
        };
        let output = &mut **output;
        let missing = Missing;
        let artifacts = self.artifacts.as_deref().unwrap_or(&missing);
        let exit = self
            .command
            .run(artifacts, ctx.timeout, ctx.cancel.clone(), output)?;
        Ok(JobOutcome {
            ok: exit.code == Some(0),
            text: None,
            wake: None,
            exit: exit.code,
        })
    }
}

struct Missing;

impl Artifacts for Missing {
    fn path_for(&self, id: i64) -> Result<PathBuf> {
        bail!("no artifact channel for background command; cannot resolve #{id}")
    }
}
