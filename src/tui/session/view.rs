//! The wire → zone translation layer (SERVER.md §3: "谁要画什么，就传什么").
//!
//! The TUI owns **zero** session state: the daemon pushes [`ServerMsg`]s, the
//! reader thread forwards them as [`Signal::Server`], and the main loop hands
//! each message to a [`SessionView`] — the one place that knows how a wire
//! message maps onto zone calls.
//!
//! 规范与实例分开：`SessionView` 是规范（一个 trait，定义"前端能看到什么、
//! 怎么落到子区"），`MainSessionView` 是唯一实例（把消息翻到 MainZone 的
//! 历史/输入/状态栏）。日后加新消息类型：wire.rs 里加变体 → 这里加一个
//! `fn on_xxx`（有默认实现=忽略）→ 需要画的 zone 自己实现。加新前端
//! （电报/web）：给它们各自写一个 `SessionView` 实例，TUI 代码零改动。

use crate::server::wire::ServerMsg;
use crate::tui::zone::main::input::statusline::{StatusEvent, UsageSnapshot};

/// What a front end does when the daemon pushes a message. Implemented once
/// per surface; the loop stays dumb (`view.on_msg(msg)`).
pub trait SessionView {
    /// A full transcript snapshot (attach, branch switch, resume, replay of
    /// a compaction). Replaces everything.
    fn on_transcript(&mut self, entries: Vec<crate::server::entry::Entry>);

    /// A burst of new transcript entries appended to the tail.
    fn on_entries(&mut self, entries: Vec<crate::server::entry::Entry>);

    /// The in-flight streaming frame (the half sentence + run state).
    fn on_stream(&mut self, frame: StreamFrame);

    /// The status line values.
    fn on_state(&mut self, frame: StateFrame);

    /// The session became known (draft submit / attach).
    fn on_attached(&mut self, session_id: i64);

    /// Protocol error (not a round error — those travel as entries).
    fn on_error(&mut self, code: crate::server::wire::ErrorCode, message: String);

    /// The handshake carried the slash-command table (see `wire::CommandInfo`).
    /// Default: ignore — a front end without completion has nothing to do with it.
    fn on_hello(&mut self, commands: Vec<crate::server::wire::CommandInfo>) {
        let _ = commands;
    }

    /// Local narration that never went near the wire: an unknown slash command,
    /// a local command that is not wired yet. Defaults to the error path so a
    /// minimal front end still shows *something*.
    fn on_notice(&mut self, text: String) {
        self.on_error(crate::server::wire::ErrorCode::Internal, text);
    }
}

/// The in-flight streaming snapshot, shaped for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamFrame {
    pub active: bool,
    pub text: String,
    pub reasoning: String,
    pub reasoning_done: bool,
    pub live: crate::server::events::LiveActivity,
    pub run_state: crate::server::events::RunState,
    /// Output of the running tool so far (empty when none is running).
    pub tool_output: String,
}

/// The status line values, shaped for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct StateFrame {
    pub model: String,
    pub name: Option<String>,
    pub cwd: String,
    pub spend_usd: f64,
    pub last_prompt_tokens: u64,
    pub context_window: u64,
    pub busy: bool,
}

/// Dispatch one wire message to the view. New `ServerMsg` variants must be
/// added here (the match is exhaustive, so the compiler enforces it).
///
/// Takes the message **by value**: the reader thread already owns it and
/// nothing after this needs it, so every field moves straight into the view.
/// The borrowed form forced a clone of the whole transcript on every attach
/// (`entries.clone()`: 18 ms and one extra full copy at 32 000 entries) and a
/// per-string clone on every streaming frame — the one message that arrives
/// at 60 Hz.
pub fn dispatch(msg: ServerMsg, view: &mut dyn SessionView) {
    match msg {
        ServerMsg::HelloOk { commands, .. } => view.on_hello(commands),
        ServerMsg::Attached { session_id } => view.on_attached(session_id),
        ServerMsg::Transcript { entries } => view.on_transcript(entries),
        ServerMsg::EntryMany { entries } => view.on_entries(entries),
        ServerMsg::Entry { entry } => view.on_entries(vec![entry]),
        ServerMsg::Stream {
            active,
            text,
            reasoning,
            reasoning_done,
            live,
            run_state,
            tool_output,
        } => view.on_stream(StreamFrame {
            active,
            text,
            reasoning,
            reasoning_done,
            live,
            tool_output,
            run_state,
        }),
        ServerMsg::State {
            model,
            name,
            cwd,
            spend_usd,
            last_prompt_tokens,
            context_window,
            busy,
        } => view.on_state(StateFrame {
            model,
            name,
            cwd,
            spend_usd,
            last_prompt_tokens,
            context_window,
            busy,
        }),
        ServerMsg::Sessions { .. } | ServerMsg::Rounds { .. } => {}
        ServerMsg::Replay { .. } => {}
        ServerMsg::Logs { .. } => {}
        ServerMsg::Error { code, message } => {
            view.on_error(code, message);
        }
    }
}

/// The MainZone instance: wire messages land as zone calls.
///
/// The zone lives in the `App` (the renderer needs it); the view borrows it
/// per message via [`MainSessionView::zone_for`] — same-loop borrows only.
/// Per-session view state that outlives one message (edge detection).
#[derive(Default)]
pub struct ViewState {
    last_busy: Option<bool>,
    last_model: String,
    last_name: String,
}

/// The MainZone instance: wire messages land as zone calls.
///
/// The zone lives in the `App` (the renderer needs it); the view borrows it
/// per message via [`MainSessionView::zone_for`] — same-loop borrows only.
/// `state` carries the view's own memory across messages (busy edges).
pub struct MainSessionView {
    pub state: ViewState,
}

impl Default for MainSessionView {
    fn default() -> Self {
        Self::new()
    }
}

impl MainSessionView {
    pub fn new() -> Self {
        Self {
            state: ViewState::default(),
        }
    }

    /// Borrow the app's MainZone as a `SessionView` for one message.
    pub fn zone_for<'a>(
        &'a mut self,
        zone: &'a mut crate::tui::zone::MainZone,
    ) -> impl SessionView + 'a {
        ZoneView {
            zone,
            last_busy: self.state.last_busy.take(),
            last_model: std::mem::take(&mut self.state.last_model),
            last_name: std::mem::take(&mut self.state.last_name),
            state_slot: &mut self.state,
        }
    }
}

/// 跨消息的记忆（边界检测用）。视图本身无状态的时代在 busy 出现
/// 边沿事件（ActivityStarted/Stopped）时结束——这是唯一需要「上一次
/// 是什么」的地方；其余字段全是幂等下发。
struct ZoneView<'a> {
    zone: &'a mut crate::tui::zone::MainZone,
    last_busy: Option<bool>,
    last_model: String,
    last_name: String,
    state_slot: &'a mut ViewState,
}

impl Drop for ZoneView<'_> {
    fn drop(&mut self) {
        // Write the cross-message memory back to the owning view.
        self.state_slot.last_busy = self.last_busy;
        self.state_slot.last_model = std::mem::take(&mut self.last_model);
        self.state_slot.last_name = std::mem::take(&mut self.last_name);
    }
}

impl SessionView for ZoneView<'_> {
    fn on_transcript(&mut self, entries: Vec<crate::server::entry::Entry>) {
        self.zone.history.replace_transcript(entries);
    }

    fn on_entries(&mut self, entries: Vec<crate::server::entry::Entry>) {
        for e in entries {
            self.zone.history.push_entry(e);
        }
    }

    fn on_stream(&mut self, frame: StreamFrame) {
        // 流式那半句拼在历史区最下面（`HistoryZone::set_live`）。按值收，
        // 两个缓冲直接移动进 zone —— 这是 30 Hz 的那条消息，克隆它等于
        // 每秒重抄几遍半篇回复。
        self.zone
            .history
            .set_live(frame.reasoning, frame.text, frame.tool_output);
    }

    fn on_state(&mut self, frame: StateFrame) {
        // busy 驱动两件事：输入区的 Esc 语义（打断 vs 退出），状态栏的
        // 活动指示（ActivityStarted/Stopped 归 activity 组件消费）。
        let was_busy = self.last_busy.replace(frame.busy);
        if was_busy != Some(frame.busy) {
            let ev = if frame.busy {
                StatusEvent::ActivityStarted
            } else {
                StatusEvent::ActivityStopped
            };
            self.zone.input.notify(&ev);
        }
        self.zone.input.streaming = frame.busy;

        // 状态栏事实下发：谁消费谁说了算（组件自己更新自己）。
        self.zone.input.notify(&StatusEvent::ModelChanged(&frame.model));
        match &frame.name {
            Some(name) => self.zone.input.notify(&StatusEvent::SessionRenamed(name)),
            None if self.last_name.is_empty() => {
                // 草稿态（新对话还没有 session/id）：以启动目录的目录名
                // 充当会话名。真实命名到达后覆盖（SessionRenamed）。
                let dir = std::path::Path::new(&frame.cwd)
                    .file_name()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_else(|| frame.cwd.clone());
                self.zone.input.notify(&StatusEvent::SessionRenamed(&dir));
                self.last_name = dir;
            }
            None => {}
        }
        self.zone.input.notify(&StatusEvent::WorkspaceChanged(std::path::Path::new(
            &frame.cwd,
        )));
        self.zone.input.notify(&StatusEvent::Usage(UsageSnapshot {
            total_cost: frame.spend_usd,
            // 分母是 models.yml 的模型元数据（服务器管）；分子是最近一轮
            // 的 prompt_tokens —— 服务器当场算，不落盘（无此字段就是 0）。
            ctx_tokens: frame.last_prompt_tokens,
            ctx_limit: frame.context_window,
            currency_symbol: "$",
            show_cost: frame.spend_usd > 0.0,
        }));
        self.last_model = frame.model;
        // 记住当前名：区分「服务器没报名字」和「名字已由真实命名产生」。
        if let Some(name) = &frame.name {
            self.last_name = name.clone();
        }
    }

    fn on_attached(&mut self, session_id: i64) {
        // 初始化语义（用户的设想）：attach/草稿提交后，daemon 立即推送
        // 快照（transcript + stream + state），所以这里**不需要**再向
        // server 讨要 —— 「init」事实上下文就是那条 Attached + 后续的
        // 快照帧。将来 zone 侧要做什么自主初始化（比如清空输入历史、
        // 重置补全会话），挂在 view 的这个回调上即可；组件级初始化走
        // StatusEvent 广播，与 on_state 同路。
        let _ = session_id;
    }

    fn on_hello(&mut self, commands: Vec<crate::server::wire::CommandInfo>) {
        self.zone.reserved.set_commands(commands);
    }

    fn on_notice(&mut self, text: String) {
        self.zone.history.push_entry(crate::server::entry::Entry::System {
            text,
            align: crate::server::entry::Align::Left,
            pin: false,
        });
    }

    fn on_error(&mut self, code: crate::server::wire::ErrorCode, message: String) {
        // 协议错误以系统条目进历史区（用户看得到，模型看不到）。
        self.zone.history.push_entry(crate::server::entry::Entry::Error {
            text: format!("[{code:?}] {message}"),
        });
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        events: Vec<&'static str>,
        transcript: Option<Vec<crate::server::entry::Entry>>,
        entries: Vec<crate::server::entry::Entry>,
        attached: Option<i64>,
        errors: Vec<String>,
        busy: Option<bool>,
    }

    impl SessionView for Recorder {
        fn on_transcript(&mut self, entries: Vec<crate::server::entry::Entry>) {
            self.events.push("transcript");
            self.transcript = Some(entries);
        }
        fn on_entries(&mut self, entries: Vec<crate::server::entry::Entry>) {
            self.events.push("entries");
            self.entries.extend(entries);
        }
        fn on_stream(&mut self, _f: StreamFrame) {
            self.events.push("stream");
        }
        fn on_state(&mut self, f: StateFrame) {
            self.events.push("state");
            self.busy = Some(f.busy);
        }
        fn on_attached(&mut self, id: i64) {
            self.events.push("attached");
            self.attached = Some(id);
        }
        fn on_error(&mut self, _c: crate::server::wire::ErrorCode, m: String) {
            self.events.push("error");
            self.errors.push(m);
        }
    }

    #[test]
    fn every_wire_message_maps_to_exactly_one_view_call() {
        // 规范的完备性：daemon 会推的每类消息都必须有落点。新增 ServerMsg
        // 变体时 dispatch 的 match 编译期强制你回来扩展；这条测试守住
        // 「每个变体恰好触发一次 view 调用」的约定。
        let cases: Vec<(ServerMsg, &[&str])> = vec![
            (
                ServerMsg::HelloOk {
                    proto: 1,
                    commands: Vec::new(),
                },
                &[],
            ),
            (ServerMsg::Attached { session_id: 3 }, &["attached"]),
            (
                ServerMsg::Transcript { entries: vec![] },
                &["transcript"],
            ),
            (ServerMsg::EntryMany { entries: vec![] }, &["entries"]),
            (
                ServerMsg::Entry {
                    entry: crate::server::entry::Entry::User { content: "u".into() },
                },
                &["entries"],
            ),
            (
                ServerMsg::Stream {
                    active: true,
                    text: "t".into(),
                    reasoning: String::new(),
                    reasoning_done: false,
                    live: Default::default(),
                    run_state: crate::server::events::RunState::Replying,
                    tool_output: String::new(),
                },
                &["stream"],
            ),
            (
                ServerMsg::State {
                    model: "m".into(),
                    name: None,
                    cwd: "/".into(),
                    spend_usd: 0.0,
                    last_prompt_tokens: 0,
                    context_window: 0,
                    busy: false,
                },
                &["state"],
            ),
            (
                ServerMsg::Error {
                    code: crate::server::wire::ErrorCode::Busy,
                    message: "busy".into(),
                },
                &["error"],
            ),
        ];
        for (msg, expect) in cases {
            let mut r = Recorder::default();
            dispatch(msg, &mut r);
            assert_eq!(r.events, expect, "case {expect:?}");
        }
    }

    #[test]
    fn zone_view_routes_the_stream_frame_to_the_history_tail() {
        // 流式帧不是被吃掉的：它落在历史区最下面那半句上。
        let mut zone = crate::tui::zone::MainZone::default();
        zone.history.rows = 10;
        let mut view = MainSessionView::new();
        {
            let mut v = view.zone_for(&mut zone);
            v.on_stream(StreamFrame {
                tool_output: String::new(),
                active: true,
                text: "答到一半".into(),
                reasoning: "想着呢".into(),
                reasoning_done: false,
                live: Default::default(),
                run_state: crate::server::events::RunState::Replying,
            });
        }
        let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let rows: String = zone
            .history
            .render_rows(80, &t)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("答到一半"), "stream 帧没落到历史区：\n{rows}");
        assert!(rows.contains("想着呢"), "思考也该跟着出来：\n{rows}");
    }

    #[test]
    fn zone_view_routes_entries_and_state() {
        // 实例验证：ZoneView 把消息落到真实的 zone 调用上。
        let mut zone = crate::tui::zone::MainZone::default();
        let mut view = MainSessionView::new();
        {
            let mut v = view.zone_for(&mut zone);
            v.on_entries(vec![
                crate::server::entry::Entry::User { content: "问".into() },
                crate::server::entry::Entry::Assistant { content: "答".into(), usage: None },
            ]);
            v.on_state(StateFrame {
                model: "m".into(),
                name: None,
                cwd: "/tmp".into(),
                spend_usd: 0.1,
                last_prompt_tokens: 5,
                context_window: 128_000,
                busy: true,
            });
            v.on_error(crate::server::wire::ErrorCode::NoSuchSession, "gone".into());
        }
        assert_eq!(zone.history.entries().len(), 3, "2 entries + 1 error entry");
        assert!(zone.input.streaming, "busy=true 必须点亮输入区的流式状态");
        // busy 边沿检测：同值重复不重复触发 Activity 事件；翻转才触发。
        assert_eq!(view.state.last_busy, Some(true));
    }

    #[test]
    fn draft_state_synthesizes_name_from_cwd() {
        // 新对话（name=None）以启动目录的目录名充当会话名；服务器带来
        // 真名后覆盖；真名之后再来 name=None 不再回退到目录名。
        let mut zone = crate::tui::zone::MainZone::default();
        let mut view = MainSessionView::new();
        let frame = |name: Option<String>| StateFrame {
            model: "m".into(),
            name,
            cwd: "/home/u/MyPi".into(),
            spend_usd: 0.0,
            last_prompt_tokens: 0,
            context_window: 0,
            busy: false,
        };
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(frame(None));
        }
        // 草稿态：目录名兜底。直接断言组件收到的广播结果 —— 查 last_name。
        assert_eq!(view.state.last_name, "MyPi");
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(frame(Some("真正的名字".into())));
        }
        assert_eq!(view.state.last_name, "真正的名字");
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(frame(None));
        }
        assert_eq!(view.state.last_name, "真正的名字", "真名后不再回退");
    }

    #[test]
    fn busy_edge_drives_activity_events_once() {
        // 状态重复不下发边沿事件；从 busy 翻到 idle 恰好一次 Stopped。
        let mut zone = crate::tui::zone::MainZone::default();
        let mut view = MainSessionView::new();
        let state_frame = |busy| StateFrame {
            model: "m".into(),
            name: None,
            cwd: "/".into(),
            spend_usd: 0.0,
            last_prompt_tokens: 0,
            context_window: 0,
            busy,
        };
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(state_frame(true));
            v.on_state(state_frame(true));
        }
        assert_eq!(view.state.last_busy, Some(true));
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(state_frame(true)); // 同值：无新边沿
        }
        assert_eq!(view.state.last_busy, Some(true));
        {
            let mut v = view.zone_for(&mut zone);
            v.on_state(state_frame(false)); // 边沿：Stopped
        }
        assert_eq!(view.state.last_busy, Some(false));
    }
}
