//! One job, one TMP file. Append while it runs; `result` reads and removes it.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use super::job::Id;
use crate::server::agent::tools::bash::bash::shell::{Exit, OutputSink};

/// The per-session output directory.
#[derive(Clone)]
pub(crate) struct OutputDir {
    root: PathBuf,
    max_output_bytes: usize,
    max_retained: usize,
}

impl OutputDir {
    pub(crate) fn new(session: i64, max_output_bytes: usize, max_retained: usize) -> Self {
        let root = crate::server::platform::env::temp_dir()
            .join("mypi-jobs")
            .join(session.to_string());
        let _ = std::fs::create_dir_all(&root);
        Self {
            root,
            max_output_bytes,
            max_retained,
        }
    }

    pub(crate) fn clean_root() {
        let root = crate::server::platform::env::temp_dir().join("mypi-jobs");
        let _ = std::fs::remove_dir_all(root);
    }

    pub(crate) fn task_dir(&self, id: Id) -> PathBuf {
        let dir = self.root.join(id.to_string());
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub(crate) fn create(&self, id: Id) -> Result<OutputFile> {
        let dir = self.task_dir(id);
        OutputFile::create(&dir, id, self.max_output_bytes)
    }

    pub(crate) fn remove_task_dir(&self, id: Id) {
        let _ = std::fs::remove_dir_all(self.root.join(id.to_string()));
    }

    pub(crate) fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }

    pub(crate) fn max_retained(&self) -> usize {
        self.max_retained
    }
}

/// One append-only output file.
pub(crate) struct OutputFile {
    path: PathBuf,
    file: std::fs::File,
    bytes: usize,
    truncated: bool,
    max: usize,
}

impl OutputFile {
    pub(crate) fn create(dir: &Path, id: Id, max: usize) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot make the job directory {}", dir.display()))?;
        let path = dir.join(format!("{id}.out"));
        let file = std::fs::File::create(&path)
            .with_context(|| format!("cannot create the output file {}", path.display()))?;
        Ok(Self {
            path,
            file,
            bytes: 0,
            truncated: false,
            max,
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn read_and_remove(mut self) -> Result<String> {
        self.file.flush()?;
        let mut text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("cannot read the output file {}", self.path.display()))?;
        if self.truncated {
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("[output truncated at {} bytes]", self.max));
        }
        let _ = std::fs::remove_file(&self.path);
        Ok(text)
    }

    pub(crate) fn discard(self) {
        let _ = std::fs::remove_file(self.path);
    }

    fn append(&mut self, text: &str) -> Result<()> {
        if self.bytes >= self.max {
            self.truncated = true;
            return Ok(());
        }
        let clean = crate::ansi::strip_ansi(text);
        let mut take = clean.len().min(self.max - self.bytes);
        while !clean.is_char_boundary(take) {
            take -= 1;
        }
        if take < clean.len() {
            self.truncated = true;
        }
        if take > 0 {
            self.file.write_all(clean[..take].as_bytes())?;
            self.bytes += take;
        }
        Ok(())
    }

    fn suffix(&mut self, text: &str) -> Result<()> {
        if self.bytes > 0 {
            self.append("\n")?;
        }
        self.append(text)
    }
}

impl OutputSink for OutputFile {
    fn out(&mut self, chunk: &str) -> Result<()> {
        self.append(chunk)
    }

    fn err(&mut self, chunk: &str) -> Result<()> {
        self.append(chunk)
    }

    fn done(&mut self, exit: &Exit) -> Result<()> {
        if exit.killed {
            if exit.canceled {
                self.suffix("[canceled]")?;
            } else {
                self.suffix(&format!(
                    "[interrupted/timeout: killed after {}s]",
                    exit.timeout_secs
                ))?;
            }
        } else if exit.code != Some(0) {
            self.suffix(&format!("[exit {}]", exit.code.unwrap_or(-1)))?;
        }
        self.file.flush()?;
        Ok(())
    }
}
