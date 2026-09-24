//! Slash commands — the `/…` table and each command's implementation.
//!
//! The functions here are `impl App` methods split into their own file:
//! they mutate the same session/editor state, but they are a separate
//! *feature* from the key-routing core. `run_command` is the only
//! dispatcher; every new command adds one arm + one method here.

use crate::ai::config::Config;
use crate::entry;
use crate::server::events::SessionEvent;
use crate::tui::app::App;

impl App {
    // Run a slash command (`cmd_name` is registered in the command table).
    //
    // Input cleanup (editor/popup/scroll) is done by the caller
    // `submit` for every command; this only performs each command's
    // business action and echo.
    pub(crate) fn run_command(&mut self, cmd_name: &str, arg: &str) {
        match cmd_name {
            "/q" | "/quit" | "/exit" => {
                // Session data is persisted at every Commit; setting the flag is enough — apply closes the main loop afterwards.
                self.quit_requested = true;
            }
            "/cdp" => self.cmd_cdp(arg),
            "/name" => self.cmd_name(arg),
            "/resume" => self.cmd_resume(),
            "/model" => self.cmd_model(arg),
            "/switch" => self.cmd_switch(arg),
            "/compact" => self.cmd_compact(arg),
            "/profile" => self.cmd_profile(arg),
            other => {
                // Any name passing lookup() must have an arm; reaching here is a programming error.
                debug_assert!(false, "未实现命令: {other}");
            }
        }
    }

    // /cdp <dir>: permanently migrate the working directory (persisted;
    // resume can restore it). Temporary migration is the AI's cd tool,
    // which never goes through here.
    pub(crate) fn cmd_cdp(&mut self, arg: &str) {
        if arg.is_empty() {
            let cur = self.session.cwd().display().to_string();
            self.session.echo(entry::Entry::Error {
                text: format!("当前工作目录：{cur}\n用法：/cdp <目录>（永久迁移，落盘）"),
            });
            return;
        }
        let target = if let Some(stripped) = arg.strip_prefix("~") {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(stripped)
        } else {
            std::path::PathBuf::from(arg)
        };
        let target = if target.is_absolute() {
            target
        } else {
            self.session.cwd().join(target)
        };
        match target.canonicalize() {
            Ok(real) if real.is_dir() => {
                let old = self.session.set_cwd(real.clone());
                let seq = self.session.bump_cwd_seq();
                self.session.handle(SessionEvent::SetCwd {
                    seq,
                    path: real.display().to_string(),
                });
                self.session.echo(entry::Entry::System {
                    text: format!("工作目录：{} → {}（已落盘）", old.display(), real.display()),
                    align: entry::Align::Center,
                });
            }
            Ok(_) => {
                self.session.echo(entry::Entry::Error {
                    text: format!("不是目录：{arg}"),
                });
            }
            Err(e) => {
                self.session.echo(entry::Entry::Error {
                    text: format!("目录不存在：{arg}（{e}）"),
                });
            }
        }
    }

    // /name [name]: name the session; no argument echoes the current name.
    //
    // The name is a tree marker (`Entry::Name`), not a session-column write:
    // branches inherit the nearest name looking back from the leaf, and
    // renaming on a branch never leaks to sibling branches.
    pub(crate) fn cmd_name(&mut self, arg: &str) {
        if arg.is_empty() {
            let cur = self.session.session_name().unwrap_or("（未命名）");
            self.session.echo(entry::Entry::Error {
                text: format!("当前会话名：{cur}。用法：/name <名字>"),
            });
            return;
        }
        // One protocol event: marker entry, persistence, legacy column —
        // all the session's business now.
        self.session
            .handle(SessionEvent::NameMarker(arg.to_string()));
        // Theme trigger: the session's color set follows its name. Same
        // name -> same theme (stable hash pick); session-temporary, the
        // config default returns next launch. Next frame repaints via the
        // per-frame Palette snapshot; the block cache drops on epoch bump.
        crate::tui::theme::rotate_for_seed(arg);
        self.session.echo(entry::Entry::System {
            text: format!(
                "已命名：{arg}（主题 {}）",
                crate::tui::theme::current_theme_name().unwrap_or_default()
            ),
            align: entry::Align::Center,
        });
    }

    // Build the resume picker entries for sessions recorded under `root`.
    // Shared by /resume and the `--resume` CLI flag (which opens the
    // picker before the first frame).
    pub(crate) fn build_resume_items(
        st: &crate::store::Store,
        root: &std::path::Path,
    ) -> anyhow::Result<Vec<(i64, String)>> {
        let metas = st.list_sessions_under(root)?;
        let items = metas
            .iter()
            .map(|m| {
                let first = st.load_entries(m.id).ok().and_then(|es| {
                    es.iter().find_map(|e| match e {
                        entry::Entry::User { content } => Some(content.clone()),
                        _ => None,
                    })
                });
                (m.id, crate::store::display_name(m, first.as_deref()))
            })
            .collect();
        Ok(items)
    }

    // /resume: list this project's sessions, stretching the reserved area.
    pub(crate) fn cmd_resume(&mut self) {
        let Some(st) = self.session.store() else {
            self.session.echo(entry::Entry::Error {
                text: "存储未打开，无法 resume".into(),
            });
            return;
        };
        let root = self.session.cwd();
        match Self::build_resume_items(st, &root) {
            Ok(items) if items.is_empty() => {
                self.session.echo(entry::Entry::Error {
                    text: format!("{} 下没有历史会话", root.display()),
                });
            }
            Ok(items) => {
                self.resume_pick = Some((items, 0));
            }
            Err(e) => {
                self.session.echo(entry::Entry::Error {
                    text: format!("读会话失败：{e:#}"),
                });
            }
        }
    }

    // /model [id]: list models or set the default (writes config.yaml; effective after restart).
    pub(crate) fn cmd_model(&mut self, arg: &str) {
        if arg.is_empty() {
            let cfg = self.cfg.as_ref().expect("cfg ready").borrow();
            let current = cfg.app.default.as_deref().unwrap_or("(未设置)");
            let mut lines = vec![format!(
                "当前默认：{current}（/model <provider>:<id> 修改，写入 config.yaml）"
            )];
            for (pname, m) in cfg.models() {
                lines.push(format!("  {pname}:{} ({})", m.id, Config::display_name(m)));
            }
            for l in lines {
                self.session.echo(entry::Entry::Error { text: l });
            }
            return;
        }
        match self
            .cfg
            .as_ref()
            .expect("cfg ready")
            .borrow()
            .model_by_id(arg)
            .ok()
        {
            Some(_) => match self
                .cfg
                .as_ref()
                .expect("cfg ready")
                .borrow()
                .save_default(arg)
            {
                Ok(()) => {
                    self.session.echo(entry::Entry::System {
                        text: format!("默认模型已设为 {arg}，已写入 config.yaml"),
                        align: entry::Align::Center,
                    });
                }
                Err(e) => {
                    self.session.echo(entry::Entry::Error {
                        text: format!("写入 config.yaml 失败：{e:#}"),
                    });
                }
            },
            None => {
                self.session.echo(entry::Entry::Error {
                    text: format!("未知模型 id：{arg}。/model 不带参数看列表"),
                });
            }
        }
    }

    // /switch [id]: switch this session's model (not persisted; restart returns to the default).
    pub(crate) fn cmd_switch(&mut self, arg: &str) {
        let cfg = self.cfg.as_ref().expect("cfg ready").borrow();
        if arg.is_empty() {
            let cur = self.current_model.borrow().display_name().to_string();
            let mut lines = vec![format!(
                "当前会话模型：{cur}（/switch <provider>:<id> 切换）"
            )];
            for (pname, m) in cfg.models() {
                lines.push(format!("  {pname}:{} ({})", m.id, Config::display_name(m)));
            }
            for l in lines {
                self.session.echo(entry::Entry::Error { text: l });
            }
            return;
        }
        match cfg.model_by_id(arg) {
            Ok(rm) => {
                drop(cfg);
                let new_model = self
                    .session
                    .switch_model(&self.cfg.as_ref().expect("cfg ready").borrow(), &rm);
                *self.current_model.borrow_mut() = new_model;
                self.session.echo(entry::Entry::System {
                    text: format!(
                        "已切换到 {} ({})，仅本会话生效",
                        arg,
                        Config::display_name(&rm.entry)
                    ),
                    align: entry::Align::Center,
                });
            }
            Err(_) => {
                self.session.echo(entry::Entry::Error {
                    text: format!("未知模型 id：{arg}。/switch 不带参数看列表"),
                });
            }
        }
    }

    // /compact [focus]: compress the finalized history into a checkpoint.
    // The summarization round-trip runs on a background thread; results
    // land via SessionEvent::Compaction. `focus` rides along as an extra
    // emphasis inside the instruction.
    pub(crate) fn cmd_compact(&mut self, arg: &str) {
        if self.session.busy() {
            self.session.echo(entry::Entry::Error {
                text: "有回合正在进行，等它结束再压缩".into(),
            });
            return;
        }
        let ccfg = {
            let cfg = self.cfg.as_ref().expect("cfg ready").borrow();
            cfg.app.compact.clone()
        };
        match self.session.run_compact(arg, &ccfg) {
            true => {
                self.session.echo(entry::Entry::System {
                    text: "正在压缩上下文…（总结请求走前缀回放，几乎只花输出费）".into(),
                    align: entry::Align::Center,
                });
            }
            false => {
                self.session.echo(entry::Entry::Error {
                    text: "压缩未能启动".into(),
                });
            }
        }
    }

    // /profile [name]: switch the system-prompt profile. Without an
    // argument, list what exists and mark the active one.
    //
    // Timing rule (the settled design): the system message is the head
    // of the cached prefix, so a switch only takes effect at a context
    // rebuild point — the next session start, or the first turn after a
    // compaction fork (entries_to_context re-reads config then). Here we
    // persist the choice; the transcript notes when it will land.
    pub(crate) fn cmd_profile(&mut self, arg: &str) {
        let cfg = self.cfg.as_ref().expect("cfg ready").borrow();
        if arg.is_empty() {
            let current = crate::server::profile::active_name(&cfg);
            let mut lines = vec![format!(
                "当前 profile：{current}（/profile <name> 切换；下次会话或压缩后生效）"
            )];
            for name in crate::server::profile::list(&cfg) {
                let mark = if name == current { " ←" } else { "" };
                lines.push(format!("  {name}{mark}"));
            }
            drop(cfg);
            for l in lines {
                self.session.echo(entry::Entry::Error { text: l });
            }
            return;
        }
        // Validate before persisting: unknown names are refused here so
        // the next session never boots into a typo.
        match crate::server::profile::resolve(&cfg, arg) {
            Ok(_) => {
                drop(cfg);
                match self
                    .cfg
                    .as_ref()
                    .expect("cfg ready")
                    .borrow()
                    .save_profile(arg)
                {
                    Ok(()) => {
                        self.session.echo(entry::Entry::System {
                            text: format!(
                                "profile 已切换为 {arg}（写入 config.yaml；下次会话或压缩后生效）"
                            ),
                            align: entry::Align::Center,
                        });
                    }
                    Err(e) => {
                        self.session.echo(entry::Entry::Error {
                            text: format!("写入 config.yaml 失败：{e:#}"),
                        });
                    }
                }
            }
            Err(e) => {
                drop(cfg);
                self.session.echo(entry::Entry::Error {
                    text: format!("{e:#}"),
                });
            }
        }
    }
}
