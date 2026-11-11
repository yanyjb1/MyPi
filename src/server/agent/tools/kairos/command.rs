//! A command admitted at tool-call time; the runner only spawns the frozen line.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::server::agent::tools::bash::bash::{admit, shell};
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::job::{Exit, JobCtx, JobOutcome, JobRunner, OutputSink};
use crate::server::platform::shell::Shell;

#[derive(Debug, Clone)]
pub(crate) struct Command {
    cwd: PathBuf,
    cmd: String,
    warn: Option<String>,
}

impl Command {
    pub(crate) fn prepare(env: &Env, content: &str) -> Result<Self> {
        let text = content.trim();
        anyhow::ensure!(!text.is_empty(), "empty command");
        let admitted = admit::admit(env, text)?;
        Ok(Self {
            cwd: env.workspace.cwd(),
            cmd: admitted.cmd,
            warn: admitted.warn,
        })
    }

    pub(crate) fn run(
        &self,
        shell: &Shell,
        timeout: Duration,
        cancel: Arc<crate::server::platform::stop::Stop>,
        output: &mut dyn OutputSink,
    ) -> Result<Exit> {
        if let Some(ids) = &self.warn {
            output.out(&format!("[warn: {ids}]\n"))?;
        }
        let mut progress = |_| {};
        shell::run_to(
            shell,
            &self.cwd,
            &self.cmd,
            timeout,
            &cancel,
            &mut progress,
            output,
        )
    }
}

pub(crate) struct CommandRunner {
    command: Command,
    shell: Shell,
}

impl CommandRunner {
    pub(crate) fn new(command: Command, shell: Shell) -> Self {
        Self { command, shell }
    }
}

impl JobRunner for CommandRunner {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        let Some(output) = ctx.output.as_mut() else {
            bail!("a command job needs an output file");
        };
        let output = &mut **output;
        let exit = self
            .command
            .run(&self.shell, ctx.timeout, ctx.cancel.clone(), output)?;
        Ok(JobOutcome {
            ok: exit.code == Some(0),
            text: None,
            wake: None,
            exit: exit.code,
        })
    }
}
