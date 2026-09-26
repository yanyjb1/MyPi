//! Where every binary mode fans out: the TUI, the foreground daemon, and
//! the one-shot queries.
//!
//! The server side owns the database and all networking (SERVER.md §0); the
//! TUI is a socket client. `mypi` with no subcommand connects to the daemon
//! and spawns one when none answers — session creation is lazy (the first
//! submit), so opening and closing the TUI writes nothing.

use mypi::server::daemon::Daemon;
use mypi::server::hub::{SessionHub, SessionSpec};
use mypi::server::socket_path;
use mypi::server::{ai::client::Client, ai::config::Config};
use mypi::cli::Cli;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;
    let mut cfg = Config::load()?;
    // --model overrides the default for this session only (never written
    // back; /model is the persistent path).
    if let Some(spec) = &cli.model {
        // Validate the id now — fail before anything starts, not on the
        // first request.
        cfg.model_by_id(spec)?;
        cfg.app.default = Some(spec.clone());
    }

    // The spec a daemon builds sessions from (provider/model/prompt/tools).
    let spec = session_spec(&cfg)?;

    if cli.server {
        return run_server_foreground(spec);
    }
    if cli.sessions {
        return mypi::oneshot::run_sessions(&spec);
    }
    if let Some((id, round)) = cli.replay {
        return mypi::oneshot::run_replay(&spec, id, round);
    }
    mypi::tui::session::run_tui(cfg, cli, spec)
}

/// Build the session spec from config: default model, its provider/key,
/// the active profile's system prompt and tool filter, tools + browser
/// settings, and the process cwd as the session's starting directory.
fn session_spec(cfg: &Config) -> anyhow::Result<SessionSpec> {
    let rm = cfg.default_model()?;
    let provider = cfg
        .models
        .providers
        .get(&rm.provider_name)
        .ok_or_else(|| anyhow::anyhow!("provider {} 未定义", rm.provider_name))?
        .clone();
    let model = rm.entry.clone();
    let api_key = cfg.resolve_key(&provider);
    let client = Client::new(&provider.base_url, &api_key, &model.id);

    // Session start is the first legal profile switch point.
    let profile_name = mypi::server::profile::active_name(cfg);
    let (system_prompt, tool_filter) =
        mypi::server::profile::resolve(cfg, &profile_name).unwrap_or_else(|e| {
            eprintln!("profile 警告：{e:#}——使用内置提示词");
            (mypi::server::profile::BUILTIN_SYSTEM.into(), None)
        });
    // The roster's source, for the one boundary that may re-read it (a
    // compaction). Same resolver as above, so "what the profile says now" has
    // exactly one definition.
    let roster_source: std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync> = {
        let cfg = cfg.clone();
        let profile_name = profile_name.clone();
        std::sync::Arc::new(move || {
            mypi::server::profile::resolve(&cfg, &profile_name)
                .map(|(_, tools)| tools)
                .unwrap_or(None)
        })
    };

    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cwd = match std::env::current_dir() {
        Ok(d) if d == home => std::env::temp_dir(),
        Ok(d) => d,
        Err(_) => std::env::temp_dir(),
    };
    // 会话级命令能碰到的东西。三个闭包各带一份配置副本：会话层不认识
    // provider、key 解析、models.yml，也不认识 config.yaml 长什么样。
    let commands = mypi::server::commands::CommandEnv {
        resolve_model: Some(std::sync::Arc::new({
            let cfg = cfg.clone();
            move |id: &str| resolve_switch(&cfg, id)
        })),
        // 用户手打的名字必须先对花名册：`profile::resolve` 是**启动**那条
        // 宽松的路（配置里写了个还没建的 profile 就当内置用），打着用户的
        // 名义悄悄"切到"一个不存在的 profile 是撒谎。
        resolve_profile: Some(std::sync::Arc::new({
            let cfg = cfg.clone();
            move |name: &str| {
                let known = mypi::server::profile::list(&cfg);
                if !known.iter().any(|n| n == name) {
                    return Err(format!("未知 profile：{name}。可用：{}", known.join("、")));
                }
                mypi::server::profile::resolve(&cfg, name).map_err(|e| format!("{e:#}"))
            }
        })),
        set_default_model: Some(std::sync::Arc::new({
            let cfg = cfg.clone();
            move |id: &str| {
                cfg.set_default_model(id)
                    .map(|p| p.display().to_string())
                    .map_err(|e| format!("{e:#}"))
            }
        })),
    };

    Ok(SessionSpec {
        client,
        system_prompt,
        max_tokens: model.max_output_tokens.unwrap_or(4096) as u32,
        cost: model.cost,
        // 模型元数据归服务器管（SERVER.md §0）：展示名有 name 用 name，
        // 没有 fallback 到 id；上下文窗口是量表分母。
        context_window: model.context_window,
        model_name: if model.name.is_empty() {
            model.id.clone()
        } else {
            model.name.clone()
        },
        cwd,
        tool_filter,
        roster_source: Some(roster_source),
        tools: cfg.app.tools.clone(),
        compact: cfg.app.compact.clone(),
        commands,
        browser: cfg.app.browser.clone(),
        // 交付节拍：config.yaml 的 app.streaming（可用 MYPI_STREAM_MODE 覆盖）。
        stream_mode: cfg.app.streaming,
    })
}

/// Resolve a model id the way `/switch` needs it: one flat struct, every
/// provider-level detail already applied (endpoint, key, display name, gauge
/// denominator, price sheet). Errors are `String` because that is what crosses
/// into a session notice.
fn resolve_switch(cfg: &Config, id: &str) -> Result<mypi::server::commands::ModelSwitch, String> {
    let rm = cfg.model_by_id(id).map_err(|e| format!("{e:#}"))?;
    let provider = cfg
        .models
        .providers
        .get(&rm.provider_name)
        .ok_or_else(|| format!("provider {} 未定义", rm.provider_name))?;
    Ok(mypi::server::commands::ModelSwitch {
        base_url: provider.base_url.clone(),
        api_key: cfg.resolve_key(provider),
        model_id: rm.entry.id.clone(),
        name: if rm.entry.name.is_empty() {
            rm.entry.id.clone()
        } else {
            rm.entry.name.clone()
        },
        context_window: rm.entry.context_window,
        max_tokens: rm.entry.max_output_tokens.unwrap_or(4096) as u32,
        cost: rm.entry.cost,
    })
}

/// `mypi --server`: bind and serve until idle-exit or quit.
fn run_server_foreground(spec: SessionSpec) -> anyhow::Result<()> {
    let db = db_path();
    let daemon = Daemon::bind(socket_path(), SessionHub::new(db), spec, idle_limit())?;
    println!("mypi daemon listening on {}", socket_path().display());
    daemon.serve()
}

/// Idle limit from MYPI_IDLE_SECS (default 600): no attached front end and
/// no running round for this long ends the daemon.
fn idle_limit() -> std::time::Duration {
    let secs = std::env::var("MYPI_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

fn db_path() -> PathBuf {
    mypi::xdg::data_dir().join("sessions.db3")
}
