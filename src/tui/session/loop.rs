//! Session lifecycle — process-level setup and the main event loop.
//!
//! APP 是 Router：接入信号、归一化、下发所有权、登记交接。
//! 渲染是 Zone 的事，这里只把 Frame 交出去。

use crate::server::ai::config::Config;
use crate::tui::app::App;
// 本地叙述（未知命令、未接线的本地命令）走 `SessionView` 的 `on_notice`。
use crate::tui::session::view::SessionView as _;
use crate::tui::zone::TermSize;
use std::sync::mpsc;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;


// Render one frame: APP 把整个屏幕交给当前所有者 Zone 自渲染。
/// 前端自己的命令：退出这类不经过服务端的事。
///
/// 表里 `scope == local` 的命令归这里；服务端只提供元数据让它认识名字。
fn local_command(
    view: &mut crate::tui::session::view::MainSessionView,
    app: &mut App,
    spec: &crate::server::commands::CommandSpec,
    args: &str,
    quit: &mut bool,
) {
    let _ = args;
    match spec.name {
        "/q" => *quit = true,
        // `/resume` 是纯前端页面：它不认识会话的"内容"，只认识列表与
        // id（列表来自服务端，附着请求发回去）。
        "/resume" => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            app.open_resume(cwd);
        }
        // 还没接线的本地命令：说清楚，别装作办了。
        other => {
            view.zone_for(&mut app.main)
                .on_notice(format!("{other} 还没接线（本地侧，下一轮）"));
        }
    }
}

/// 补全要用的模型候选：`provider:id` + 灰色那行的显示名。
fn model_candidates(
    cfg: &Config,
) -> Vec<crate::tui::zone::main::reserved::completion::controller::ModelCandidate> {
    cfg.models()
        .map(
            |(provider, m)| crate::tui::zone::main::reserved::completion::controller::ModelCandidate {
                provider: provider.to_string(),
                id: m.id.clone(),
                detail: Config::display_name(m).to_string(),
            },
        )
        .collect()
}

/// 把选择器排队的事发出去：拉列表 / 附着 / 删除。
///
/// 附着之后所有权已经交回主区（Zone 自己交的），这里只管把消息发上路。
/// 启动路径与主循环都调它——`--resume` 的第一帧不能是空白。
fn send_resume_requests(app: &mut App, req: &mut crate::server::wire::ConnWriter) {
    for r in app.take_resume_requests() {
        match r {
            crate::tui::zone::resume::Request::List { under } => {
                req.request(&crate::server::wire::ClientMsg::ListSessions { under })
                    .ok();
            }
            crate::tui::zone::resume::Request::Attach(id) => {
                req.request(&crate::server::wire::ClientMsg::Attach { id }).ok();
            }
            crate::tui::zone::resume::Request::Delete(id) => {
                req.request(&crate::server::wire::ClientMsg::DeleteSession { id })
                    .ok();
            }
        }
    }
}

fn draw_frame(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> Result<()> {
    terminal.draw(|f| {
        // Zone 自渲染：行高仲裁、子区布局、内容全部 Zone 内部完成。
        use crate::tui::zone::Zone as _;
        let owner = app.current_owner();
        let lines = match owner {
            crate::tui::zone::ZoneId::Main => app.main.render(),
            crate::tui::zone::ZoneId::Resume => app.resume.render(),
            crate::tui::zone::ZoneId::Tree => Vec::new(), // 会话树 Zone 未实现
        };
        f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
        // 硬件光标：输入区报容器内的位置，主区换算成终端坐标（带钳制）。
        // 不设置就是隐藏——ratatui 每帧默认藏光标，除非这一帧明确要位置。
        // 只有主区的输入区会要光标；选择器的搜索框不放硬件光标
        // （那一页自己画 `> ` 提示）。
        if owner == crate::tui::zone::ZoneId::Main
            && let Some((x, y)) = app.main.cursor_position()
        {
            f.set_cursor_position((x, y));
        }
    })?;
    Ok(())
}

// Run the TUI (blocking; Esc / Ctrl+C exits).
pub fn run_tui(
    cfg: Config,
    cli: crate::cli::Cli,
    spec: crate::server::hub::SessionSpec,
) -> Result<()> {
    let _ = (&cli, &spec); // the spec built the daemon's session factory
    // Theme init: config.yaml 的 theme.name 指定 JSON 主题。
    let cfg_theme = cfg.app.theme.clone();
    crate::tui::theme::init_from_config(cfg_theme.name.as_deref(), Some(&cfg_theme));

    // ---- server-first cutover: the TUI owns ZERO session state ----
    // The connection lives in the reader thread below; its drop (when the
    // loop dies) detaches us — the daemon's cue to stop a running round
    // (SERVER.md §4).
    let mut conn = connect_daemon(&spec)?;
    // Attach from the CLI (`mypi attach <id>`), else stay in the draft
    // state: no session row until the first submit.
    if let Some(id) = cli.attach {
        conn.request(&crate::server::wire::ClientMsg::Attach { id })?;
        // The reply carries the whole transcript: bound it by a deadline that
        // fits a long session, not by the socket's poll interval.
        let attached = conn.wait_for_within(
            |m| matches!(m, crate::server::wire::ServerMsg::Attached { .. }),
            crate::server::wire::SNAPSHOT_DEADLINE,
        )?;
        if let crate::server::wire::ServerMsg::Attached { session_id } = attached {
            eprintln!("attached to session {session_id}");
        }
    }

    // ---- terminal setup ----
    let mut terminal = ratatui::init();
    let _ = execute!(
        std::io::stdout(),
        ratatui::crossterm::event::PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
        )
    );
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    // 启动尺寸契约：量一次，交给 Zone，之后只在 resize 时广播。
    let size = terminal.size()?;
    // 启动就进哪一页由命令行定：`mypi --resume` 直接开选择器，否则主区。
    // 两个 Zone 都在 `App` 里建好（都拿到启动尺寸），交接只是换账本。
    let startup = if cli.resume {
        crate::tui::zone::ZoneId::Resume
    } else {
        crate::tui::zone::ZoneId::Main
    };
    let mut app = App::new(
        TermSize {
            cols: size.width,
            rows: size.height,
        },
        startup,
    );
    // 补全服务上岗：保留区的第一个住户。cwd/home 是会话层注入的
    // 最后一项（之后路径解析全在服务内部）。
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    app.main
        .reserved
        .attach_completion(std::env::current_dir().unwrap_or_else(|_| home.clone()), home);
    // 命令表来自握手（`hello_ok`）：前端不再自己存一份。补全服务上岗之后再喂。
    app.main.reserved.set_commands(conn.commands().to_vec());
    // 参数的**合法值**：命令表只说形状（`model_id` / `profile_name`），
    // 名字得有人送。它们都是运行时配置事实（不是会话状态），而配置这一份
    // 进程已经加载了——状态栏的模型名读的就是它。
    app.main
        .reserved
        .set_candidates(model_candidates(&cfg), crate::server::profile::list(&cfg));

    // 状态栏自初始化（TUI 本地事实，不烦服务器）：默认模型显示名从
    // 进程启动已加载的 Config 解析（name 字段，缺省回退 id），cwd 就是
    // 进程启动目录。首个服务端 State 帧（submit 后）到达时以活值覆盖。
    {
        use crate::tui::zone::main::input::statusline::StatusEvent;
        let model_name = cli
            .model
            .as_deref()
            .and_then(|id| cfg.model_by_id(id).ok())
            .map(|rm| rm.entry.display_name().to_string())
            .or_else(|| {
                cfg.app
                    .default
                    .as_deref()
                    .and_then(|id| cfg.model_by_id(id).ok())
                    .map(|rm| rm.entry.display_name().to_string())
            })
            .unwrap_or_default();
        // 还没有 session id 的草稿态：先叫「新会话」。**只在内存里**——
        // 服务端要到第一次 submit 才建行，这个名字不落任何盘。真实名字
        // （`/name` 或由首条消息合成）随第一个 State 帧覆盖它。
        app.main
            .input
            .notify(&StatusEvent::SessionRenamed("新会话"));
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        app.main.input.notify(&StatusEvent::ModelChanged(&model_name));
        app.main
            .input
            .notify(&StatusEvent::WorkspaceChanged(&cwd));
    }

    // `--resume`：启动就站在选择器上，并且**立刻**去拉列表——不然第一帧
    // 是一片空白，要等用户敲个键才有内容。
    if cli.resume {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        app.open_resume(cwd);
    }

    // ---- signal pump ----
    let (sig_tx, sig_rx) = mpsc::channel::<crate::tui::session::signal::Signal>();

    // Producer 3: the workspace git poller (session view 层的约定：分支是
    // 外部实时事实，前端组件自己算——SERVER.md §0)。5s 一拍；快照作为
    // 信号进主循环，语义（消费/隐藏/着色）全在 git 组件。
    let sig_tx_git = sig_tx.clone();
    let git_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    std::thread::spawn(move || loop {
        let snap = crate::git::snapshot(&git_dir);
        if sig_tx_git
            .send(crate::tui::session::signal::Signal::Git(snap))
            .is_err()
        {
            return; // main loop gone
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    });

    // Writer handle for the main loop: issued BEFORE the connection moves
    // into the reader thread. Both share one socket (see the request-site
    // comment below for why there is exactly one connection).
    let mut req = conn.writer();

    // Producer 2: the daemon's pushes (socket reader thread). The connection
    // handle lives in this thread only; the main loop receives signals.
    let sig_tx_server = sig_tx.clone();
    let mut server_conn = Some(conn);
    std::thread::spawn(move || {
        if let Some(mut c) = server_conn.take() {
            loop {
                match c.read_msg() {
                    Ok(msg) => {
                        if sig_tx_server
                            .send(crate::tui::session::signal::Signal::Server(msg))
                            .is_err()
                        {
                            return; // main loop gone
                        }
                    }
                    Err(e) if crate::server::wire::is_read_timeout(&e) => {
                        // Poll interval expired on an idle connection: not a
                        // hangup. Keep waiting; the daemon stays silent when
                        // nothing happens.
                        continue;
                    }
                    Err(e) => {
                        // Two different failures wear the same face here. A
                        // hangup means the daemon is gone; a decode failure
                        // means the daemon said something this build cannot
                        // read (an oversized line, a protocol drift). Saying
                        // "连接已断开" for the second one sends the reader
                        // hunting for a dead process that is alive and well.
                        let what = match e.kind() {
                            std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionReset => "与 daemon 的连接已断开",
                            _ => "收到无法解析的消息，已停止接收",
                        };
                        let _ = sig_tx_server.send(
                            crate::tui::session::signal::Signal::Server(
                                crate::server::wire::ServerMsg::Error {
                                    code: crate::server::wire::ErrorCode::Internal,
                                    message: format!("{what}：{e}"),
                                },
                            ),
                        );
                        return;
                    }
                }
            }
        }
    });

    // Producer 1: keyboard/mouse signals.
    let sig_tx_p1 = sig_tx.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            let fwd = match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    Some(crate::tui::session::signal::Signal::Key(k))
                }
                Event::Paste(s) => Some(crate::tui::session::signal::Signal::Paste(s)),
                Event::Mouse(m) => Some(crate::tui::session::signal::Signal::Mouse(m)),
                Event::Resize(_w, _h) => Some(crate::tui::session::signal::Signal::Resized),
                _ => None,
            };
            if let Some(s) = fwd
                && sig_tx_p1.send(s).is_err()
            {
                break; // main loop gone
            }
        }
    });


    // SessionView 的实例状态（busy 边沿检测的记忆）。规范在
    // `SessionView` trait，实例在 `MainSessionView::zone_for`——主循环
    // 只搬运消息，语义全部下沉到 view 与组件。
    let mut view = crate::tui::session::view::MainSessionView::new();

    let result = (|| -> Result<()> {
        // 进循环之前先把选择器（`--resume`）的请求发掉：第一帧要有内容，
        // 而主循环要等到有信号才会走到发送那一步。
        send_resume_requests(&mut app, &mut req);
        // Initial paint.
        draw_frame(&mut terminal, &mut app)?;
        let mut quit = false;
        while !quit {
            // Block until something happens; coalesce queued signals.
            let first = sig_rx.recv()?;
            let mut batch = Vec::new();
            batch.push(first);
            while let Ok(more) = sig_rx.try_recv() {
                batch.push(more);
            }

            'outer: for sig in batch {
                match sig {
                    crate::tui::session::signal::Signal::Key(k) => {
                        app.deliver(crate::tui::keys::normalize(k));
                    }
                    crate::tui::session::signal::Signal::Paste(s) => {
                        app.deliver(crate::tui::zone::RawEvent::Paste(s));
                    }
                    crate::tui::session::signal::Signal::Mouse(m) => {
                        // 只归一化滚轮；其余鼠标输入丢弃（keys::normalize_mouse）。
                        if let Some(ev) = crate::tui::keys::normalize_mouse(&m) {
                            app.deliver(ev);
                        }
                    }
                    crate::tui::session::signal::Signal::Resized => {
                        let s = terminal.size()?;
                        app.on_resize(TermSize {
                            cols: s.width,
                            rows: s.height,
                        });
                    }
                    crate::tui::session::signal::Signal::Server(msg) => {
                        // 会话列表是**选择器**的输入，不是主区的：谁持有
                        // 所有权就归谁（APP 只认账本，不认消息类型）。
                        if app.current_owner() == crate::tui::zone::ZoneId::Resume {
                            match msg {
                                crate::server::wire::ServerMsg::Sessions { sessions } => {
                                    app.resume.on_sessions(sessions);
                                    continue;
                                }
                                crate::server::wire::ServerMsg::Error { message, .. } => {
                                    app.resume.on_error(message);
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        {
                            // 服务端推送 → SessionView（规范）→ zone 调用（实例）。
                            // 规范/实例分离见 view.rs 模块注释。
                            // Ownership passes straight through: reader thread →
                            // signal → here → view, no copy of the transcript.
                            crate::tui::session::view::dispatch(
                                msg,
                                &mut view.zone_for(app.main_mut()),
                            );
                        }
                    }
                    crate::tui::session::signal::Signal::Git(snap) => {
                        // 外部实时事实 → 状态栏广播。借用所有权随信号移动，
                        // 组件自己决定克隆与隐藏。
                        app.main_mut()
                            .input
                            .notify(&crate::tui::zone::main::input::statusline::StatusEvent::Git(
                                snap.as_ref(),
                            ));
                    }
                }
                // 出口请求：编辑器声明"要出去"，这里执行。
                for x in app.take_outcomes() {
                    match x {
                        crate::tui::zone::main::input::ExitRequest::Quit => {
                            quit = true;
                            break 'outer;
                        }
                        crate::tui::zone::main::input::ExitRequest::Submit(text) => {
                            // 斜杠命令在**这里**只做识别：表来自服务端（`hello_ok`
                            // 带下来），去向按表的 `scope` 分——会话命令走 wire，
                            // 本地命令当场办。以前整串文本无条件当用户消息发给
                            // 模型，十个命令一个都不生效。
                            match crate::server::commands::split(&text) {
                                Some((spec, args)) => {
                                    match spec.scope {
                                        crate::server::commands::Scope::Session => {
                                            req.request(&crate::server::wire::ClientMsg::Command {
                                                name: spec.name.to_string(),
                                                args: args.to_string(),
                                            })
                                            .ok();
                                        }
                                        crate::server::commands::Scope::Local => {
                                            local_command(&mut view, &mut app, spec, args, &mut quit)
                                        }
                                    }
                                }
                                // 未知的 `/xxx`：明确告知，不静默发给模型
                                // （那既浪费 token，又让模型对着一句它执行不了的
                                // 指令瞎猜）。
                                None if text.trim_start().starts_with('/') => {
                                    let word = text.split_whitespace().next().unwrap_or("");
                                    view.zone_for(&mut app.main)
                                        .on_notice(format!("未知命令 {word}（Tab 补全可看全部）"));
                                }
                                None => {
                                    // Draft submit: the daemon creates the session
                                    // row and answers `attached` (SERVER.md §1).
                                    req.request(&crate::server::wire::ClientMsg::Submit {
                                        text: text.clone(),
                                    })
                                    .ok();
                                }
                            }
                        }
                        crate::tui::zone::main::input::ExitRequest::Interrupt => {
                            req.request(&crate::server::wire::ClientMsg::Interrupt).ok();
                        }
                    }
                }
                // 选择器要主循环替它做的事（见函数）。
                send_resume_requests(&mut app, &mut req);
            }

            // One repaint per coalesced batch.
            draw_frame(&mut terminal, &mut app)?;
        }
        Ok(())
    })();

    let _ = execute!(
        std::io::stdout(),
        ratatui::crossterm::event::PopKeyboardEnhancementFlags
    );
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}


/// Connect to the daemon, spawning one when no daemon answers. Blocking;
/// the TUI calls this once, before the first frame.
fn connect_daemon(
    spec: &crate::server::hub::SessionSpec,
) -> Result<crate::server::wire::ClientConn> {
    let _ = spec;
    let path = crate::server::socket_path();
    if let Ok(c) = crate::server::wire::ClientConn::connect(&path) {
        return Ok(c);
    }
    // Spawn a detached daemon (our own binary in --server mode) and retry.
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("--server")
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("无法拉起 mypi daemon: {e}"))?;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        if let Ok(c) = crate::server::wire::ClientConn::connect(&path) {
            return Ok(c);
        }
    }
    anyhow::bail!("daemon 未就绪：{}", path.display())
}
