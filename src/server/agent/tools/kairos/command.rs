//! A command frozen at tool-call time; stored texts resolved only when it runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};

use crate::server::agent::tools::Env;
use crate::server::agent::tools::Export;
use crate::server::agent::tools::bash::bash::shell::{self, OutputSink};
use crate::server::agent::tools::bash::bash::{expand_alias, guard, rewrite_refs};
use crate::server::agent::tools::jobs::{JobCtx, JobOutcome, JobRunner, OutputFile};
use crate::server::platform::shell::Shell;

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
        if env.bash.shell.guarded()
            && let guard::Verdict::Block(hits) = guard::classify(&template, &zone)
        {
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
        shell: &Shell,
        export: &dyn Export,
        timeout: std::time::Duration,
        cancel: Arc<AtomicBool>,
        output: &mut OutputFile,
    ) -> Result<shell::Exit> {
        let cmd = rewrite_refs(&self.template, export, shell)?;
        if shell.guarded() {
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
        }
        let mut progress = |_| {};
        shell::run_to(
            shell,
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
    shell: Shell,
    export: Option<Arc<dyn Export>>,
}

impl CommandRunner {
    pub(crate) fn new(command: Command, shell: Shell, export: Option<Arc<dyn Export>>) -> Self {
        Self {
            command,
            shell,
            export,
        }
    }
}

impl JobRunner for CommandRunner {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        let Some(output) = ctx.output.as_mut() else {
            bail!("a command job needs an output file");
        };
        let output = &mut **output;
        let missing = NoExport;
        let export = self.export.as_deref().unwrap_or(&missing);
        let exit =
            self.command
                .run(&self.shell, export, ctx.timeout, ctx.cancel.clone(), output)?;
        Ok(JobOutcome {
            ok: exit.code == Some(0),
            text: None,
            wake: None,
            exit: exit.code,
        })
    }
}

/// No export channel: a background command has no stored text to reach for.
struct NoExport;

impl Export for NoExport {
    fn dir(&self) -> &Path {
        Path::new(".")
    }

    fn path(&self, reference: &str) -> Result<PathBuf> {
        bail!("no export channel for a background command; cannot read #{reference}")
    }
}
