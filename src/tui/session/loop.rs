//! Session lifecycle — process-level setup and the main event loop.
//!
//! Split from the `App` state machine (app.rs): that file answers "what
//! does this keypress mean"; this one owns the terminal session — opening
//! the store (with legacy-DB migration), driving the signal pump, calling
//! the renderer, and placing the hardware cursor.

use crate::ai::client::Client;
use crate::ai::config::Config;
use crate::ai::types::{Context as ChatContext, Message};
use crate::tui::app::{App, SPIN_INTERVAL};
use crate::tui::keys::Action;
use crate::tui::keys::translate_with;
use crate::tui::layout as tlayout;
use crate::tui::theme::Palette;
use crate::tui::view::{self, ViewState};
use std::sync::mpsc;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyboardEnhancementFlags, MouseEventKind,
};
use ratatui::crossterm::execute;

use super::{db_path, migrate_legacy_db};
use crate::tui::app::display_name;

// Render one frame: sizes, scroll correction, widget drawing, hardware
// cursor placement. Shared by the pre-loop first paint and the signal
// pump — a stale first frame (or none at all) is how "blank until a
// keystroke" bugs happen.
fn draw_frame(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    palette: &mut Palette,
    ctx_limit: u64,
    currency_symbol: &'static str,
    show_cost: bool,
) -> Result<crate::tui::layout::Layout> {
    // Per-frame snapshot: a /theme-style runtime switch lands on the next
    // frame with zero plumbing (omp's "bump epoch, repaint" contract).
    *palette = Palette::current();
    let size = terminal.size()?;
    let wrapped = app.wrapped(size.width);
    let cwd_str = app.session.cwd().display().to_string();
    let cursor_char = app.editor.cursor();
    let spinner = app.spinner();

    // Viewport scrolling: minimal-displacement correction based on the
    // previous frame's viewport start. The height must be the one
    // **after subtracting the reserved area**, matching view's layout,
    // or the two sides disagree and the scroll window misaligns.
    let ch = tlayout::container_height(
        size.height.saturating_sub(app.reserved_height(size.height)),
        wrapped.len(),
    );
    let cursor_row = wrapped.locate(cursor_char).0;
    app.scroll = tlayout::adjust_scroll(app.scroll, wrapped.len(), ch, cursor_row);

    // Layout is computed once: `app` and `view` share the same sizes.
    let l = tlayout::compute(
        size.height,
        &wrapped,
        app.scroll,
        app.reserved_height(size.height),
        crate::tui::components::reserved::DEFAULT_MAX as u16,
    );
    let mut cursor_pos = (0u16, 0u16);
    let model_name = Config::display_name(&app.current_model.borrow()).to_string();
    let session_name = display_name(app);
    terminal.draw(|f| {
        // Modal takeover: the tree navigator draws over the whole
        // screen; base zones and the hardware cursor are skipped.
        if let Some(tp) = app.tree_pick.as_ref() {
            let lines =
                crate::tui::components::tree_picker::render(tp, size.width, size.height, palette);
            f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            return;
        }
        let mut vs = ViewState {
            history: app.session.transcript(),
            transcript_generation: app.session.transcript_generation(),
            block_cache: &mut app.block_cache,
            chat_scroll: app.history.chat_scroll,
            scroll_pinned: app.history.scroll_pinned,
            show_reasoning: !app.history.reasoning_folded,
            tools_expanded: app.history.tools_expanded,
            // The live row is driven by the session, the only party that knows
            // whether the server is thinking or a tool is running.
            live: &app.session.stream_view().live,
            streaming: {
                let sv = app.session.stream_view();
                if sv.text.is_empty() {
                    None
                } else {
                    Some(sv.text.as_str())
                }
            },
            wrapped: &wrapped,
            cursor_char,
            spinner,
            model_name: &model_name,
            session_name: &session_name,
            cwd: &cwd_str,
            git: app.git.as_ref(),
            ctx_tokens: app.tracker.last_prompt_tokens,
            ctx_limit,
            cost: app.tracker.total,
            currency_symbol,
            show_cost,
            palette: *palette,
            popup: app.completion.popup(),
            resume_pick: app.resume_pick.as_ref().map(|(v, i)| (&v[..], *i)),
        };
        cursor_pos = view::draw(f, &mut vs, &l);
    })?;

    // ---- hardware cursor ----
    // `Layout::cursor_y` guarantees the cursor stays inside the input
    // container and never tramples the reserved area; ratatui/crossterm
    // do no boundary checks, so this is the only gate.
    terminal.set_cursor_position(ratatui::layout::Position {
        x: cursor_pos.1.min(size.width.saturating_sub(1)),
        y: l.cursor_y(size.height, cursor_pos.0),
    })?;
    terminal.show_cursor()?;
    Ok(l)
}

// Run the TUI (blocking; Esc / Ctrl+C exits).
pub fn run_tui(cfg: Config, cli: crate::cli::Cli) -> Result<()> {
    // Session-scoped mutable config: /model and /switch both change the
    // current model, hence RefCell. Main thread only (Rc is not Send);
    // the background turn thread gets its own cloned Client.
    let cfg = std::rc::Rc::new(std::cell::RefCell::new(cfg));
    let rm = cfg.borrow().default_model()?;
    let provider = cfg
        .borrow()
        .models
        .providers
        .get(&rm.provider_name)
        .ok_or_else(|| anyhow::anyhow!("provider {} 未定义", rm.provider_name))?
        .clone();
    let model = rm.entry.clone();
    let api_key = cfg.borrow().resolve_key(&provider);
    let client = Client::new(&provider.base_url, &api_key, &model.id);
    let cost_cfg = model.cost;
    let currency_symbol = model.currency.symbol();
    // Theme init: config.yaml 的 theme.name 指定 JSON 主题（内置 titanium/dark
    // 或 ~/.config/mypi/themes/<name>.json）；未配置时用内置 titanium。
    // models.yml 的 accent/gold 覆盖仅在其偏离默认值时生效（旧配置兼容）。
    let cfg_theme = cfg.borrow().app.theme.clone();
    crate::tui::theme::init_from_config(cfg_theme.name.as_deref(), Some(&cfg_theme));
    let mut palette = Palette::current();
    // Cost is computed **locally**: the gateway only reports token counts
    // (prompt/completion/cached), the program multiplies by the unit
    // prices the user wrote in models.yml. All-zero prices = "no price
    // sheet for this model" -> hide the spend column (there is nothing
    // meaningful to compute, not "the gateway didn't quote").
    let show_cost =
        (cost_cfg.input + cost_cfg.output + cost_cfg.cache_read + cost_cfg.cache_write) > 0.0;
    let current_model = std::rc::Rc::new(std::cell::RefCell::new(model.clone()));

    // Session start is the first legal profile switch point: resolve the
    // system prompt (and tool roster) from the configured profile. A
    // broken/unreachable profile dir degrades to the built-in prompt —
    // never block startup on cosmetics.
    let profile_name = crate::server::profile::active_name(&cfg.borrow());
    let (system_prompt, tool_filter) =
        crate::server::profile::resolve(&cfg.borrow(), &profile_name).unwrap_or_else(|e| {
            eprintln!("profile 警告：{e:#}——使用内置提示词");
            (crate::server::profile::BUILTIN_SYSTEM.into(), None)
        });
    let chat = ChatContext::new().push(Message::System {
        content: system_prompt,
    });
    // Workspace license: launching from $HOME would license the whole
    // home directory for rm/mv — exactly what the guard exists to
    // prevent. Degrade to /tmp instead; the user can /cd out of it.
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cwd = match std::env::current_dir() {
        Ok(d) if d == home => std::env::temp_dir(),
        Ok(d) => d,
        Err(_) => std::env::temp_dir(),
    };
    // Session service facade: owns turn resources + SessionState + the
    // event channel. The TUI holds it and the rx.
    let max_tokens = model.max_output_tokens.unwrap_or(4096) as u32;
    let (session, rx) = crate::server::Session::new(
        {
            migrate_legacy_db(&crate::xdg::data_dir());
            crate::server::SessionState::new(crate::store::Store::open(&db_path()).ok())
        },
        client,
        chat,
        max_tokens,
        cost_cfg,
        cwd.clone(),
        tool_filter,
        cfg.borrow().app.tools.clone(),
        Some(db_path()),
    );
    let mut app = App::new(session, current_model);
    app.cfg = Some(cfg.clone());
    // `--resume`: open the session picker before the first frame (the
    // same surface /resume shows; Esc here simply starts a fresh session).
    if cli.resume
        && let Some(st) = app.session.store()
        && let Ok(items) = App::build_resume_items(st, &app.session.cwd())
        && !items.is_empty()
    {
        // No store / no sessions: fall through to a fresh session.
        app.resume_pick = Some((items, 0));
    }
    let ctx_limit = model.context_window;

    let mut terminal = ratatui::init();
    // `ratatui::init` installs a panic hook that only disables raw mode
    // and leaves the alternate screen — it knows nothing about the three
    // modes **we** enable below (mouse capture, bracketed paste, the
    // Kitty keyboard protocol). Without this chain a panic leaves the
    // terminal reporting every mouse move as an escape sequence pasted
    // into the shell. Restore order: undo our modes *first*, then defer
    // to ratatui's hook (raw mode / alt screen).
    let restore_full = |prev: Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send>| {
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(
                std::io::stdout(),
                ratatui::crossterm::event::PopKeyboardEnhancementFlags
            );
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            let _ = execute!(std::io::stdout(), DisableBracketedPaste);
            prev(info);
        }));
    };
    restore_full(std::panic::take_hook());
    // Kitty keyboard protocol: ask the terminal to disambiguate escape
    // sequences so **Shift+Enter** arrives as Enter+SHIFT (newline) and
    // bare Enter as plain Enter (submit). Terminals without the
    // protocol silently ignore the escape codes — the pre-existing
    // fallbacks (Alt+Enter / Ctrl+J) keep working there.
    let _ = execute!(
        std::io::stdout(),
        ratatui::crossterm::event::PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        )
    );
    // Enable bracketed paste: the terminal wraps pasted content in
    // \x1b[200~ ... \x1b[201~, so we get Event::Paste instead of every
    // line arriving as keystrokes.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    // Enable mouse capture: wheel events scroll the history area. Side
    // effect: native text selection usually needs Shift+drag.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    let result = (|| -> Result<()> {
        // Last frame's layout, for mouse zone hit-testing.
        let mut last_layout: Option<crate::tui::layout::Layout> = None;

        // Paint the first frame **before** blocking: the loop below is
        // signal-driven, and without an initial draw the user stares at
        // a blank screen until the first keystroke arrives.
        draw_frame(
            &mut terminal,
            &mut app,
            &mut palette,
            ctx_limit,
            currency_symbol,
            show_cost,
        )?;

        // ---- signal pump: input thread + session events ----
        //
        // The loop is **signal-driven, not polled**: it blocks on
        // `sig_rx.recv()` and only wakes when something actually
        // happened. Two producers feed the loop:
        //   input thread  — forwards raw crossterm events as `Signal`s
        //   session drain — SessionEvents drained below, between signals
        // A burst of deltas marks the dirty bit repeatedly but repaints
        // once — coalescing is free because dirty is idempotent.
        let (sig_tx, sig_rx) = mpsc::channel::<crate::tui::session::signal::Signal>();
        // A second Sender handle for the session-event forwarder below.
        let sig_tx2 = sig_tx.clone();

        // Producer 1: raw terminal input, forwarded as signals.
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                let fwd = match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        Some(crate::tui::session::signal::Signal::Key(k))
                    }
                    Event::Paste(s) => Some(crate::tui::session::signal::Signal::Paste(s)),
                    Event::Mouse(m) => Some(crate::tui::session::signal::Signal::Mouse(m)),
                    Event::Resize(_, _) => Some(crate::tui::session::signal::Signal::Resized),
                    _ => None,
                };
                if let Some(s) = fwd
                    && sig_tx.send(s).is_err()
                {
                    break; // main loop gone
                }
            }
        });

        // Producer 2: session events (deltas, tool calls, commits).
        // Forwarding them through the SAME channel is what makes the
        // loop truly signal-driven: a delta arriving wakes the loop and
        // repaints the live slot. (An earlier draft kept a separate
        // `rx` and drained it only after *keyboard* signals — the
        // stream never reached the screen unless the user typed.)
        std::thread::spawn(move || {
            for ev in rx {
                if sig_tx2
                    .send(crate::tui::session::signal::Signal::Session(ev))
                    .is_err()
                {
                    break; // main loop gone
                }
            }
        });

        let mut quit = false;
        while !quit {
            // Block until *something* happens. During streaming the
            // deltas themselves are the wake-ups; no POLL interval, no
            // idle wake-ups. Everything already queued behind the first
            // signal is folded into the same batch: N queued deltas =
            // one repaint, not N (coalescing for free).
            // The wait doubles as the spinner's heartbeat: while a turn
            // is streaming (including silent thinking phases, which
            // produce no deltas) wake at SPIN_INTERVAL to keep it
            // spinning; when idle block forever — no timers, no CPU.
            let first = if app.session.busy() {
                match sig_rx.recv_timeout(SPIN_INTERVAL) {
                    Ok(s) => Some(s),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                }
            } else {
                Some(sig_rx.recv()?)
            };
            let mut batch = Vec::new();
            if let Some(s) = first {
                batch.push(s);
                // Fold everything already queued behind the first signal:
                // N queued deltas = one repaint, not N (coalescing free).
                while let Ok(more) = sig_rx.try_recv() {
                    batch.push(more);
                }
            }

            // ---- route the batch ----
            let mut batch_tools_ran = false;
            for sig in batch {
                match sig {
                    crate::tui::session::signal::Signal::Key(k) => {
                        let term_w = terminal.size()?.width;
                        let action = translate_with(k, app.key_context(term_w));
                        quit = !app.apply(action, term_w);
                    }
                    crate::tui::session::signal::Signal::Paste(s) => {
                        // Paste also goes through `apply`.
                        //
                        // This used to be a **second** path: a direct
                        // `app.editor.insert_paste()` plus two lines of
                        // hand-copied epilogue, bypassing `apply`. It
                        // missed `input_history.on_edit()` — pasting
                        // while browsing history stuck in browse mode,
                        // and the next ↑ jumped to the entry before last
                        // instead of saving a draft. The two paths
                        // agreeing was pure luck; unified now, no drift
                        // possible.
                        let term_w = terminal.size()?.width;
                        quit = !app.apply(Action::Paste(s), term_w);
                    }
                    crate::tui::session::signal::Signal::Mouse(m) => {
                        // Hit-test the pointer row against the last frame's
                        // layout: only the history area scrolls (input and
                        // reserved ignore the wheel). Up unpin; back at 0
                        // re-pins. A modal turns the wheel into list scroll.
                        let chat_h = last_layout
                            .as_ref()
                            .map(|l: &crate::tui::layout::Layout| l.chat_height)
                            .unwrap_or(0);
                        if crate::tui::zones_impl::wheel_zone(
                            m.row,
                            chat_h,
                            app.resume_pick.is_some(),
                        ) == Some(crate::tui::zones::ZoneId::History)
                        {
                            match m.kind {
                                MouseEventKind::ScrollUp => {
                                    crate::tui::zones_impl::wheel_step(&mut app.history, true, 3)
                                }
                                MouseEventKind::ScrollDown => {
                                    crate::tui::zones_impl::wheel_step(&mut app.history, false, 3)
                                }
                                _ => {}
                            }
                        }
                    }
                    crate::tui::session::signal::Signal::Resized => {
                        // Widths changed: every cached wrap is invalid. The
                        // recompute below always reads terminal.size(), so
                        // nothing else to do — the repaint below is
                        // unconditional after any signal.
                    }
                    crate::tui::session::signal::Signal::Session(ev) => {
                        if app.session.ingest(ev) == crate::server::events::Change::ToolActivity {
                            batch_tools_ran = true;
                        }
                    }
                }
            }

            // Environment checkpoint (event-driven): a tool result just
            // landed (tools are the only things that can move the working
            // tree — plain speech never triggers a refresh) or the cwd
            // moved. The 2s rate limit inside `refresh_env` absorbs
            // multi-tool bursts in a single round.
            if batch_tools_ran || app.env_cwd != app.session.cwd() {
                app.refresh_env();
            }

            // One spinner frame per loop pass: delta batches repaint
            // anyway, and SPIN_INTERVAL timeouts keep it alive through
            // silent thinking phases.
            app.advance_spinner();

            // ---- render one frame (shared with the pre-loop first draw) ----
            let l = draw_frame(
                &mut terminal,
                &mut app,
                &mut palette,
                ctx_limit,
                currency_symbol,
                show_cost,
            )?;
            last_layout = Some(l);
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

// Turn machinery (spawn runner, collect_turn, entries_to_context) lives
// in `crate::server::turn` now — the TUI is only a subscriber.
