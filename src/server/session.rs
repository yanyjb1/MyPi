//! Session state — the **server-side** conversation model, free of TUI
//! dependencies (no ratatui, no event enums). The TUI subscribes to it:
//! it mutates via the narrow mutators below and renders from
//! `transcript()` / `snapshot()`.
//!
//! Owns the "session viscera" that used to sit directly on `App`:
//! transcript, pending round entries, store handle, session id, name,
//! cwd sequence. Cross-cutting flows — command echoes, turn commits,
//! tree navigation, /name markers — all go through here, so the same
//! rules apply no matter which surface triggered them.

use anyhow::{Context as _, anyhow};
use crate::server::ai::client::Client;
use crate::server::ai::types::{Context as ChatContext, Usage};
use crate::server::entry::Entry;
use crate::server::events::{Change, LiveActivity, RunState, SessionEvent, StreamView};

/// How much of a **running** tool's output the live slot keeps.
///
/// The tail is what matters mid-command ("has it printed the error yet?"), and
/// the finished result carries everything anyway — so this is a display buffer,
/// not a transcript: bounded, so a command printing megabytes costs the same as
/// one printing a screenful.
const TOOL_OUTPUT_TAIL: usize = 16 * 1024;
use crate::server::store::{RoundMeta, Store};
use std::sync::{Arc, Mutex};

pub struct SessionState {
    // Rendered entries (in-memory + DB-resumed share one path).
    transcript: Vec<Entry>,
    // Bumped whenever the transcript is **replaced** wholesale (tree
    // navigation, resume) rather than appended to. The render cache keys
    // on this: a swap to a same-length branch is invisible to a length
    // check, so the rows of the branch you left would keep painting.
    transcript_generation: u64,
    // Entries produced this round; verified/persisted at TurnDone (Commit).
    pending: Vec<Entry>,
    /// 惰性历史：resume 只装了尾巴，完整转录还没读进来（见 `hub::resume`）。
    /// 第一回合之前必须补齐——否则模型看到的上下文只有尾巴。
    history_pending: bool,
    /// 见 [`Self::blocks_clean`]。
    blocks_clean: bool,
    // Storage. None = DB unavailable (degrades to in-memory session).
    store: Option<Store>,
    // Current session id; None until the first turn (session created lazily).
    session_id: Option<i64>,
    // Name set explicitly via /name; None = statusline synthesizes one.
    session_name: Option<String>,
    // ---- streaming slots (owned here; the TUI only reads StreamView) ----
    stream: StreamView,
    // Last turn's usage (TurnDone), consumed by the caller's cost tracker.
    last_usage: Option<Usage>,
    // Last turn's usage snapshot, parked for `Commit` to fold into the reply
    // entry the runner assembles (which carries no usage of its own).
    pending_usage: Option<Usage>,
    // Whether the current round's buffers have already been committed to the
    // transcript. Set by `finalize_round`, cleared by `start_turn` and by every
    // event that puts new data into the buffers — that is what makes a second
    // finalize for the *same* round a no-op while still letting a round that
    // received more data after an error be stored.
    round_finalized: bool,
    // The round in flight's request header (model / endpoint / system prompt /
    // tool manuals / token ceiling). Sent by the turn runner **before** its
    // first request and written inside the same transaction as the round's
    // entries, so even a round that dies mid-flight is stored complete with
    // what it was asking.
    round_meta: Option<RoundMeta>,
    // How the round in flight ended (`StopReason::as_str`); None while it is
    // live or when it died before the gateway answered. Written with the round.
    round_stop: Option<&'static str>,
    /// 当前模型属于哪个服务商（models.yml 的段名）。`Session` 在装配与
    /// `/switch` 时写，落盘按轮记它——同名模型可以挂两家，账单要认得出谁是谁。
    model_provider: String,
    /// 转录**还没落盘**的那截尾巴：正在跑的那个回合（`pending` 里的那些）加上
    /// 永远不落盘的回声（通知、命名标记、落盘失败报告）。
    ///
    /// 它是前端窗口的对侧账本：前端只留一个有界的块窗口，所以"哪些还是活的、
    /// 没有块 id"必须由这里说了算。不变量：`live` 就是 `transcript` 的尾巴，
    /// 顺序一致（每个 push 都成对走 [`Self::push_live`]）。
    live: Vec<Entry>,
}

impl SessionState {
    pub fn new(store: Option<Store>) -> Self {
        Self {
            transcript: Vec::new(),
            transcript_generation: 0,
            pending: Vec::new(),
            history_pending: false,
            blocks_clean: true,
            store,
            session_id: None,
            session_name: None,
            stream: StreamView::default(),
            last_usage: None,
            pending_usage: None,
            round_finalized: false,
            round_meta: None,
            round_stop: None,
            model_provider: String::new(),
            live: Vec::new(),
        }
    }

    /// 推一条**还没落盘**的转录尾巴：入 `transcript` 也入 `live`。
    ///
    /// 只有两种条目走这里：这个回合正在攒的（`pending` 的同一份）和永远不落盘
    /// 的回声。落盘的条目在写成功后由 [`Self::note_persisted`] 从 `live` 里摘掉。
    fn push_live(&mut self, e: Entry) -> Change {
        self.live.push(e.clone());
        self.transcript.push(e);
        Change::Transcript
    }

    /// 落盘即作废的条目：只有未钉住的系统通知。它们说的是"刚刚发生了什么"
    /// （切了模型、换了目录、压缩了上下文），话说完就该退场——块一落盘，
    /// 屏幕上留着的只是过期的告示。钉住的不算（`pin` 的语义就是"常住"），
    /// 其余条目也不是（它们都有落盘的那一天）。
    fn dies_on_persist(e: &Entry) -> bool {
        matches!(e, Entry::System { pin: false, .. })
    }

    /// 落盘成功后：把这批条目从 `live` 里摘掉（按顺序做子序列匹配，不是比长度
    /// ——回合中间可能夹着回声）。
    ///
    /// 返回**顺序是否干净**：真 = 被摘掉的恰好是 `live` 的前缀，剩下的都排在
    /// 它们后面，前端可以拿"整段替换 live"来对齐；假 = 有回声夹在中间，前端
    /// 必须改收一次整体快照（daemon 那边据此决定发哪条消息）。
    ///
    /// 匹配用相等判断就够：落盘的条目只有 User/Assistant/Reasoning/Tool*/Todo，
    /// 回声只有 System/Error/Name，两边的 `kind` 不相交，不会摘错。
    ///
    /// 顺手把**落盘即作废**的条目一并撤掉（见 [`Self::dies_on_persist`]）。
    fn note_persisted(&mut self, written: &[Entry]) -> bool {
        let mut j = 0;
        let mut prefix = true;
        let mut kept: Vec<Entry> = Vec::with_capacity(self.live.len());
        for e in self.live.drain(..) {
            if j < written.len() && written[j] == e {
                j += 1;
                continue;
            }
            // 撤掉的短命通知不算"顺序脏"：它不在留下的序列里，前端拿到整段
            // 替换的 `live` 照样能对齐（少一次全量快照）。
            if Self::dies_on_persist(&e) {
                continue;
            }
            prefix = prefix && j >= written.len();
            kept.push(e);
        }
        self.live = kept;
        prefix && j == written.len()
    }

    // ---- event intake (the protocolized write side) ----

    /// Consume one protocol event, in order. This is the *only* path a
    /// turn's data takes into the session; input surfaces and the turn
    /// runner both speak [`SessionEvent`]. Returns what changed so the
    /// renderer reacts without learning how.
    ///
    /// Usage lands on `last_usage` (read via [`take_last_usage`]); the
    /// caller owns the model's price sheet and does the local pricing —
    /// the session stays pricing-agnostic.
    pub fn handle(&mut self, ev: SessionEvent) -> Change {
        match ev {
            SessionEvent::Delta(d) => {
                self.round_finalized = false; // new data: the round is live again
                self.stream.reasoning_done = true; // content started; reasoning frozen
                self.stream.text.push_str(&d);
                // Content is arriving: it occupies the live row itself.
                self.stream.live = LiveActivity::Idle;
                Change::Stream
            }
            SessionEvent::ReasoningDelta(r) => {
                self.round_finalized = false; // new data: the round is live again
                // First reasoning chunk of this turn means the server has
                // started talking — and it is talking about its own thoughts
                // rather than answering. That is exactly "thinking".
                if self.stream.reasoning.is_empty() {
                    self.stream.live = LiveActivity::Thinking;
                }
                self.stream.reasoning.push_str(&r);
                Change::Stream
            }
            SessionEvent::Todo { phases } => {
                // State, not narration: the entry renders as nothing, and what
                // matters is that the *last* one is the current list (resume,
                // branch switch, and the context note all read it that way).
                let e = Entry::Todo { phases };
                self.pending.push(e.clone());
                self.push_live(e)
            }
            SessionEvent::ToolProgress { call_id, chunk } => {
                // `call_id` is not used for routing: only one tool runs at a
                // time (the loop is serial), so the live slot is unambiguous.
                // It rides along for front ends that want to pair it up.
                let _ = call_id;
                self.stream.tool_output.push_str(&chunk);
                if self.stream.tool_output.len() > TOOL_OUTPUT_TAIL {
                    let cut = self.stream.tool_output.len() - TOOL_OUTPUT_TAIL;
                    let cut = (cut..self.stream.tool_output.len())
                        .find(|i| self.stream.tool_output.is_char_boundary(*i))
                        .unwrap_or(cut);
                    self.stream.tool_output.drain(..cut);
                    self.stream.tool_output.insert_str(0, "…\n");
                }
                Change::Stream
            }
            SessionEvent::ToolStart {
                call_id,
                name,
                args,
                intent,
                text,
                first,
            } => {
                self.round_finalized = false; // new data: the round is live again
                self.stream.live = LiveActivity::Tool {
                    intent: intent.clone(),
                };
                // A new call starts with a clean slate: the previous call's
                // output belongs to its own (now finished) card.
                self.stream.tool_output.clear();
                // The message's text and its opening call are captured here;
                // the round's own text buffer is cleared because it belongs
                // to this tool message now, not to the final reply (else the
                // two would merge into one reply on screen and on disk).
                if first && !text.is_empty() {
                    self.stream.text.clear();
                }
                // 这段话说之前的**思考**先落成条目：它的位置就在这个工具
                // 调用**之前**。以前只有 `finalize_round` 会把思考缓冲折成
                // 条目，于是"思考 → 调工具 → 再思考 → 正文"在屏幕上变成
                // "工具 → 工具 → 思考 → 正文"——顺序整个倒了。
                // （`first` 只标记本条消息的第一个调用，思考也只属于那一处。）
                if first {
                    let reasoning = std::mem::take(&mut self.stream.reasoning);
                    if !reasoning.is_empty() {
                        let r = Entry::Reasoning { content: reasoning };
                        self.pending.push(r.clone());
                        self.push_live(r);
                    }
                }
                let e = Entry::ToolRequest {
                    call_id,
                    name,
                    args,
                    intent,
                    text,
                    first,
                };
                self.pending.push(e.clone());
                self.push_live(e)
            }
            SessionEvent::ToolFinish {
                call_id,
                name,
                ok,
                result,
                details,
                duration_ms,
            } => {
                self.round_finalized = false; // new data: the round is live again
                // The call this row described has landed; the next event
                // (another tool, or the reply) decides what replaces it. The
                // live output goes with it — the result entry carries the real
                // thing now.
                self.stream.live = LiveActivity::Idle;
                self.stream.tool_output.clear();
                let e = Entry::ToolResult {
                    call_id,
                    name,
                    ok,
                    // Store data only: the model-facing text, plus whatever
                    // structured payload the tool produced. How either is drawn
                    // is the front end's business, never this layer's.
                    result,
                    details,
                    duration_ms,
                };
                self.pending.push(e.clone());
                self.push_live(e);
                Change::ToolActivity
            }
            SessionEvent::Error(e) => {
                // Mid-flight death (network drop, malformed stream) or a
                // session-level notice. If a turn was streaming, whatever it
                // produced is finalized **and persisted** first: a dropped
                // connection must not cost the user the partial reply they
                // already watched arrive. The error itself is display-only
                // (not a protocol message kind).
                if self.stream.active {
                    self.finalize_round(None);
                }
                self.push_live(Entry::Error { text: e })
            }
            SessionEvent::TurnDone(u, stop) => {
                // The single assembly point: whatever the event stream built
                // up in `pending` IS the round. No second assembler runs, so
                // there is nothing to keep in sync and no lossy projection
                // between what streamed and what is stored.
                //
                // The stop reason is recorded too: without it a round the user
                // interrupted and a round that finished are the same bytes on
                // disk, and "what did the model do" becomes unanswerable.
                self.round_stop = Some(stop.as_str());
                self.finalize_round(Some(u.clone()));
                self.pending_usage = Some(u.clone());
                self.last_usage = Some(u);
                Change::TurnDone
            }
            SessionEvent::Done => {
                self.stream.active = false;
                Change::Stream
            }
            SessionEvent::Submit(_) => {
                // Turn *starting* is the surface's job (it owns the turn
                // runner and the Client); the session only records the
                // resulting entries. Nothing to consume here.
                Change::None
            }
            SessionEvent::NameMarker(name) => {
                // Marker entry (tree semantics) + best-effort legacy column.
                self.session_name = Some(name.clone());
                let marker = Entry::Name { name: name.clone() };
                self.push_live(marker.clone());
                // Borrow discipline: store writes in scoped blocks; error
                // echoes go into the transcript only after the borrow ends.
                let write_err = self
                    .persistence()
                    .and_then(|(st, sid)| st.append(sid, std::slice::from_ref(&marker)).err());
                if let Some(e) = write_err {
                    self.push_live(Entry::Error {
                        text: format!("命名写入失败：{e:#}"),
                    });
                }
                if let Some((st, sid)) = self.persistence() {
                    let _ = st.set_session_name(sid, Some(&name));
                }
                Change::Transcript
            }
            SessionEvent::RequestMeta {
                model,
                protocol,
                base_url,
                system,
                tools_json,
                max_tokens,
                compat_json,
            } => {
                // Parked, not displayed and not part of the transcript: it is
                // the *request*, which no transcript row represents. Written
                // with the round at finalize time.
                self.round_meta = Some(RoundMeta {
                    model,
                    provider: self.model_provider.clone(),
                    protocol,
                    base_url,
                    system,
                    tools_json,
                    max_tokens,
                    compat_json,
                });
                Change::None
            }
            SessionEvent::SetCwd { path } => {
                // Bookkeeping only: nothing to draw. Storage keys the migration
                // on the session's current tip block.
                if let Some((st, sid)) = self.persistence() {
                    let _ = st.record_cwd(sid, &path);
                }
                Change::None
            }
            SessionEvent::Compaction { .. } => {
                // Intercepted by `Session::ingest` before reaching here —
                // the facade owns the chat replica this event replaces.
                Change::None
            }
        }
    }

    /// The **single** assembly point for a round. Folds the streaming
    /// buffers into entries (reasoning first, then the reply riding usage),
    /// then persists everything `pending` accumulated this round and clears
    /// it. Called at TurnDone *and* on a mid-flight Error, so a dropped
    /// connection stores exactly what streamed instead of losing the round.
    ///
    /// `usage` is `None` on the error path (the stream died before the
    /// final usage chunk arrived); the reply entry is still written, just
    /// without a stats line.
    ///
    /// **Idempotent per round**: the round-limit brake sends `Error` and then
    /// `TurnDone` for the same round, so this runs twice. The second pass would
    /// find the buffers already drained and append a phantom `EMPTY_REPLY`
    /// reply — stored in the DB, so the user would see a bogus bubble after
    /// every brake. Any event that puts new data in the buffers re-opens the
    /// round (see `round_finalized`).
    fn finalize_round(&mut self, usage: Option<Usage>) {
        if self.round_finalized {
            return;
        }
        self.round_finalized = true;
        let content = std::mem::take(&mut self.stream.text);
        let content = if content.is_empty() {
            crate::server::entry::empty_reply().into()
        } else {
            content
        };
        let reasoning = std::mem::take(&mut self.stream.reasoning);
        if !reasoning.is_empty() {
            let r = Entry::Reasoning { content: reasoning };
            self.pending.push(r.clone());
            self.push_live(r);
        }
        let round_usage = usage.as_ref().map(|u| crate::server::store::RoundUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cached_tokens: u.cached_tokens.unwrap_or(0),
        });
        let e = Entry::Assistant {
            content,
            usage: usage.map(|u| Entry::usage_summary(&u)),
        };
        self.pending.push(e.clone());
        self.push_live(e);
        self.stream.active = false;

        // Take the round out before persisting: `persistence()` borrows
        // `self` mutably and the store write must not alias the buffer.
        let round = std::mem::take(&mut self.pending);
        let meta = self.round_meta.take();
        let stop = self.round_stop.take();
        if let Some((st, sid)) = self.persistence() {
            // Entries **and** the request header go in one transaction: a
            // stored round is either complete (transcript + what was asked) or
            // absent. That is what makes the DB alone sufficient to reproduce
            // the conversation byte for byte, with no profile/config lookup.
            let res = match &meta {
                Some(m) => st.append_round(sid, &round, m, stop, round_usage),
                // No header seen (in-memory callers, older front ends): store
                // the entries anyway rather than lose the round.
                None => st.append(sid, &round),
            };
            match res {
                // 写进去了：这些条目从"还没落盘"变成块（id 下来了，daemon 那边
                // 据 `blocks_clean` 决定是发块增量还是整体快照）。
                Ok(_) => {
                    let clean = self.note_persisted(&round);
                    self.blocks_clean = self.blocks_clean && clean;
                }
                Err(e) => {
                    let msg = format!("落盘失败：{e:#}");
                    self.push_live(Entry::Error { text: msg });
                }
            }
        }
    }

    /// 抛下一轮：流式槽位、回合缓冲、收尾凭证全部归零，**不落盘**。
    ///
    /// 与 [`Self::finalize_round`] 相对：那个**收尾**（把这一轮存下来），这个
    /// **丢弃**（这一轮不属于任何人了）。分叉用它——被抛下的那一轮属于旧
    /// 分支，既不该继续占着 `busy()`（新分支要能立刻开工），也不该把它的
    /// 半截回复写进新分支的转录。
    pub fn abandon_turn(&mut self) {
        self.stream = StreamView::default();
        // 收尾闸门关死：旧回合线程随后飘来的 TurnDone / Error 到这里只会
        // 提前返回。事件通道那边已经换掉了（见 `Session::abandon_turn`），
        // 这是第二道闸。
        self.round_finalized = true;
        self.round_meta = None;
        self.round_stop = None;
        self.pending.clear();
        self.pending_usage = None;
    }

    /// The last turn's usage (set by TurnDone). Cleared on read; the
    /// caller folds it into its cost tracker.
    pub fn take_last_usage(&mut self) -> Option<Usage> {
        self.last_usage.take()
    }

    /// Read-only view of the streaming slots (renderer subscription).
    /// The model's todo list as it stands: the **last** `Entry::Todo` in the
    /// transcript. Empty when the tool was never used.
    ///
    /// Reading it off the transcript (rather than keeping a parallel copy) is
    /// what makes resume and branch switches work for free: the list travels
    /// with the conversation.
    pub fn current_todo(&self) -> Vec<crate::server::entry::TodoPhase> {
        let from_entries = self.transcript().iter().rev().find_map(|e| match e {
            Entry::Todo { phases } => Some(phases.clone()),
            // 兜底：`Entry::Todo` 是**回合结束**才落的，而卡片（工具结果）在调用
            // 当场就落库了。回合中途死掉（网络断、被 Esc 打断）时，备忘录不能
            // 跟着丢——所以从最后一条 `todo` 工具结果的 `details` 里再捞一次。
            Entry::ToolResult {
                details: Some(d), ..
            } if d.get("kind").and_then(|k| k.as_str()) == Some("todo") => d
                .get("phases")
                .cloned()
                .and_then(|p| serde_json::from_value(p).ok()),
            _ => None,
        });
        from_entries.unwrap_or_default()
    }

    pub fn stream_view(&self) -> &StreamView {
        &self.stream
    }

    /// Is a turn currently streaming? (Surfaces check this before
    /// starting a new one.)
    pub fn busy(&self) -> bool {
        self.stream.active
    }

    /// Start a turn: echo + stage the user message and reset the
    /// streaming slots. Called by the surface right before spawning the
    /// turn runner — one protocol action instead of three field pokes.
    pub fn start_turn(&mut self, user_text: &str) -> Change {
        self.round_finalized = false; // a fresh round: nothing committed yet
        self.round_meta = None; // a fresh round: its header is not known yet
        self.round_stop = None;
        let e = Entry::User {
            content: user_text.to_string(),
        };
        self.push_live(e.clone());
        self.pending.push(e);
        self.stream = StreamView {
            active: true,
            ..Default::default()
        };
        Change::Stream
    }

    // ---- read side (renderer subscription) ----

    pub fn transcript(&self) -> &[Entry] {
        &self.transcript
    }

    /// Generation of the current transcript (0 = never replaced). The render
    /// cache compares this across frames to notice a wholesale swap.
    pub fn transcript_generation(&self) -> u64 {
        self.transcript_generation
    }

    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    pub fn session_id(&self) -> Option<i64> {
        self.session_id
    }

    /// Rebuild one stored round's request **from the database alone**.
    ///
    /// This is the contract that makes a session reproducible from storage:
    /// system prompt, tool manuals, model id, endpoint and token ceiling all
    /// come from the round row, never from the local config or the active
    /// profile. A different machine, a different UI (a web front end, a chat
    /// bridge) or an edited `config.yaml` therefore replays the same bytes.
    ///
    /// `messages` is the context as it stood the moment the round ended — the
    /// exact prefix the next round's request would carry. `body` is that same
    /// request's JSON, produced by the very function the live client uses
    /// (`ai::client::body_json`), so it cannot drift from what was sent.
    ///
    /// Errors instead of guessing: an unreadable entry or tool manual means the
    /// bytes cannot be reproduced, and silently skipping a row would hand back
    /// a *nearly* right request — worse than none, because it looks right.
    pub fn replay_round(&self, session_id: i64, round_seq: i64) -> Result<Replay, String> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| "数据库不可用，无法复现".to_string())?;
        let row = store
            .round(session_id, round_seq)
            .map_err(|e| format!("读请求头失败：{e:#}"))?
            .ok_or_else(|| format!("会话 {session_id} 没有第 {round_seq} 回合的请求头"))?;
        // The branch as of *that* turn's tip: replaying after a fork must
        // rebuild the conversation that was live then, not today's branch.
        let (entries, unreadable) = store
            .load_branch(session_id, row.last_block)
            .map_err(|e| format!("读条目失败：{e:#}"))?;
        if !unreadable.is_empty() {
            return Err(format!(
                "有 {} 块读不出来（block {:?}），无法逐字节复现",
                unreadable.len(),
                unreadable
            ));
        }
        let tools: serde_json::Value = serde_json::from_str(&row.tools_json)
            .map_err(|e| format!("工具定义读不回来：{e}"))?;
        let ctx = crate::server::turn::entries_to_context(&row.system, &entries);
        // The per-request ceiling is derived, not stored: the loop derives it
        // the same way from the same rebuilt context, so the value matches.
        let max_tokens = crate::server::agent::loop_rs::LoopConfig::new(row.max_tokens)
            .effective_max_tokens(&ctx);
        // Half the switches are re-derived and half are stored, and the split
        // is the point: detection is a pure function of (base_url, model id),
        // both of which the round row carries; the user's `compat:` block is
        // not recoverable once the config is edited, so it was stored. An
        // unparsable block is an error rather than a silent fallback to
        // detection — that would hand back a *nearly* right request, which
        // looks right and is worse than none.
        let user_compat: Option<crate::server::ai::compat::Compat> = match row.compat_json.as_deref() {
            None => None,
            Some(json) => Some(serde_json::from_str(json).map_err(|e| {
                format!("第 {} 回合的 compat 块读不回来（{e}），无法逐字节复现", row.seq)
            })?),
        };
        let compat = crate::server::ai::compat::resolve(&row.base_url, &row.model, user_compat.as_ref());
        // Same two-step as the live path: the wire shape of the messages array
        // depends on the host, so it is built from the switches, not straight
        // from the internal types.
        let messages = crate::server::ai::assemble::messages_to_wire(&ctx.messages, &compat);
        // 会话 id 同时喂两个地方：`prompt_cache_key`（请求体）和会话亲和头。
        // 这里必须用**同一个**字符串，否则 replay 出来的字节和当时发出去
        // 的对不上——而"逐字节复现"正是这个函数存在的理由。
        let session_key = session_id.to_string();
        let body = crate::server::ai::assemble::body_json(
            &row.model,
            max_tokens,
            true,
            &messages,
            &tools,
            &compat,
            Some(&session_key),
        );
        Ok(Replay {
            round: row.seq,
            model: row.model,
            protocol: row.protocol,
            base_url: row.base_url,
            system: row.system,
            tools_json: row.tools_json,
            stop_reason: row.stop_reason,
            max_tokens,
            messages,
            body,
        })
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    pub fn store_mut(&mut self) -> Option<&mut Store> {
        self.store.as_mut()
    }

    pub fn pending_snapshot(&self) -> &[Entry] {
        &self.pending
    }

    // ---- write side (narrow mutators) ----

    /// Append an echo/status entry to the transcript (never persisted).
    pub fn echo(&mut self, e: Entry) -> Change {
        self.push_live(e)
    }

    /// Queue a round entry into `pending` (persisted at TurnDone).
    pub fn stage(&mut self, e: Entry) {
        self.pending.push(e);
    }

    /// Commit a finalized round: replace the transcript with `entries`
    /// (which include the merged pending tail), clear pending, and if a
    /// session exists persist everything not yet in the store.
    pub fn commit_round(&mut self, entries: Vec<Entry>) {
        self.replace_transcript(entries);
        self.pending.clear();
    }

    /// Swap in a whole new transcript and bump the generation so the render
    /// cache drops the previous one. `replace_transcript` is the *only*
    /// path that assigns `transcript` — one place to forget the bump.
    fn replace_transcript(&mut self, entries: Vec<Entry>) {
        self.transcript = entries;
        // 整体替换来的条目全部是**库里读出来的**（resume / 补齐 / 分支跳转），
        // 所以"还没落盘的尾巴"清空；块顺序也重来（各前端都会收到整体快照）。
        self.live.clear();
        self.blocks_clean = true;
        self.transcript_generation = self.transcript_generation.wrapping_add(1);
    }

    /// 把一条**已经落盘**的条目推进转录（不走 `live`）。
    ///
    /// 只有压缩用：分隔标记是先写库、再上屏的，进 `live` 的话它会被当成"还没
    /// 落盘的尾巴"再发一遍，和 daemon 随后按块 id 发的那份撞成两条。
    fn push_stored(&mut self, e: Entry) -> Change {
        self.transcript.push(e);
        Change::Transcript
    }

    /// 还没落盘的那截尾巴（daemon 发块增量时一并带给前端）。
    pub fn live(&self) -> &[Entry] {
        &self.live
    }

    /// 自上次整体快照以来，每次落盘是不是都"顺序干净"（被写掉的条目恰好排在
    /// 剩余条目的前面）。假的含义：有一声回声夹在回合中间，前端不能靠"整段替换
    /// live"对齐 → daemon 改发一次整体快照（见 [`Self::note_persisted`]）。
    pub fn blocks_clean(&self) -> bool {
        self.blocks_clean
    }

    /// daemon 发过一次整体快照之后，账本回到干净（各前端都重新对齐了）。
    pub fn reset_blocks_clean(&mut self) {
        self.blocks_clean = true;
    }

    /// Create a fresh session (or adopt an existing one) and adopt the
    /// given entries as the transcript. Returns the session id.
    pub fn adopt_session(&mut self, id: i64, entries: Vec<Entry>, name: Option<String>) -> Change {
        self.session_id = Some(id);
        self.replace_transcript(entries);
        self.session_name = name;
        Change::Session
    }

    /// 就地把这条会话挪到**另一行**（分叉）。
    ///
    /// 与 `adopt_session` 的区别：那个是"换成另一段对话"，这个是**同一段
    /// 对话换一条分支**——内存态一概不动，只改 `session_id` 和转录前缀。
    /// 改完之后写块归新分支（`persistence()` 读的就是这个 id）。
    pub fn retarget(&mut self, id: i64, entries: Vec<Entry>) {
        self.session_id = Some(id);
        self.replace_transcript(entries);
    }

    /// Record the effective session name (resume path).
    pub fn set_session_name(&mut self, name: Option<String>) {
        self.session_name = name;
    }

    /// Try to create the session lazily on the first turn. Returns the
    /// new id, or None when the store is unavailable (in-memory mode).
    pub fn ensure_session(&mut self, root: &std::path::Path) -> Option<i64> {
        if self.session_id.is_some() {
            return self.session_id;
        }
        let st = self.store.as_mut()?;
        match st.create_session(&crate::server::store::now_stamp(), &root.display().to_string()) {
            Ok(id) => {
                self.session_id = Some(id);
                Some(id)
            }
            Err(e) => {
                self.push_live(Entry::Error {
                    text: format!("会话创建失败：{e:#}"),
                });
                None
            }
        }
    }

    /// Store + id, paired — most persistence flows need both.
    pub fn persistence(&mut self) -> Option<(&mut Store, i64)> {
        let sid = self.session_id?;
        Some((self.store.as_mut()?, sid))
    }

    /// 惰性历史：尾巴之后还有更老的条目没进来。
    pub fn history_pending(&self) -> bool {
        self.history_pending
    }

    /// resume 时登记"只装了尾巴"（第一回合之前要补齐）。
    pub fn set_history_pending(&mut self, pending: bool) {
        self.history_pending = pending;
    }
}

// ---- facade ------------------------------------------------------------

/// A stored round's request, rebuilt from the database alone.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Replay {
    /// Which round this is (1-based, per session).
    pub round: i64,
    pub model: String,
    /// Wire protocol id the request spoke.
    pub protocol: String,
    pub base_url: String,
    /// System prompt verbatim, as it entered the request.
    pub system: String,
    /// Tool manuals verbatim (serialized JSON array).
    pub tools_json: String,
    /// How the round ended; `None` = died before the gateway answered.
    pub stop_reason: Option<String>,
    /// Token ceiling the rebuilt body carries.
    pub max_tokens: u32,
    /// The context as it stood when the round ended (the next request's prefix).
    pub messages: serde_json::Value,
    /// The request body JSON, built by the live client's own serializer.
    pub body: serde_json::Value,
}

/// High-level session handle — the object a surface *holds*.
///
/// Owns everything a turn needs (client, shared chat replica, cwd slot,
/// interrupt flag, price sheet) and exposes protocol-level operations:
/// `submit` spawns the turn runner and routes its events into the
/// [`SessionState`]. A surface (TUI or a headless driver) never touches
/// turn viscera — it calls methods here and drains `Change`s.
pub struct Session {
    pub state: SessionState,
    client: std::cell::RefCell<Client>,
    chat: Arc<Mutex<ChatContext>>,
    max_tokens: u32,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    // snapshots at start, the surface writes; the RwLock keeps reads and
    // writes from trampling each other.
    pub cwd: Arc<std::sync::RwLock<std::path::PathBuf>>,
    // Where this session started: the file tools' boundary, fixed for the life
    // of the session. `cwd` migrates (cd); this is what "inside the workspace"
    // means, so it must not move with it.
    workspace_root: std::path::PathBuf,
    // Price sheet of the current model (local cost computation).
    pub cost: crate::server::ai::config::Cost,
    // Context window of the current model (models.yml): the usage gauge's
    // denominator on the front end. 0 = unknown (gauge hides).
    context_window: u64,
    // Statusline display name (models.yml `name`, else the id). Display-only.
    model_name: String,
    // 计价货币：金额的符号从它来（`¥` / `$` / `cr`）。和模型同寿命。
    currency: crate::server::ai::config::Currency,
    cost_tracker: crate::server::ai::pricing::CostTracker,
    tx: std::sync::mpsc::Sender<SessionEvent>,
    // Profile tool roster (see `TurnRequest.tool_filter`). Set at
    // startup from the active profile; the profile switch command only
    // re-reads it at the next rebuild point.
    tool_filter: Option<Vec<String>>,
    // Where the roster comes from, when the profile may be re-read at a legal
    // boundary (`tools.reloadOnCompaction`). `None` = the roster is whatever it
    // was at construction, for the session's whole life.
    //
    // A source function rather than a path: the session does not know whether
    // profiles come from files, a config section or a remote — it only knows
    // that "the roster may be asked for again".
    roster_source: Option<std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync>>,
    // Tool-layer knobs from config.yaml (`tools:`). Snapshotted per turn;
    // a config edit takes effect on the next turn.
    tools: crate::server::ai::config::ToolsConfig,
    // Compaction knobs (config.yaml → `compact:`), needed by `/compact` — the
    // one place a *command* starts background work.
    compact: crate::server::ai::config::CompactConfig,
    // What session-scoped commands may reach outside the session (a model
    // resolver, a profile root, the config file). Empty = the commands say
    // "not wired" instead of pretending (see `run_command`).
    commands: crate::server::commands::CommandEnv,
    // Browser settings for the web tools (config.yaml → `browser:`).
    browser: crate::server::ai::config::BrowserConfig,
    // Session DB path (file-backed store): the turn thread opens its
    // own connection from here to spill/fetch artifacts.
    artifact_db: Option<std::path::PathBuf>,
    // How streamed text reaches the subscriber (see `config::StreamMode`).
    // Read when a round starts, so a switch never splits one round's output
    // between two modes.
    stream_mode: crate::server::ai::config::StreamMode,
    // 开场白体重的缓存（`context_estimate`）：算一次要查库 + 折字符，而状态帧
    // 每跳都可能来问一次。`None` = 还没算过。
    estimate: std::cell::Cell<Option<u64>>,
    // 模型不可用就把会话卡在这里（`restore_model` 解析失败）：不静默降级，
    // 换不到模型就别接着烧钱。`Some` = 拒绝开工，内容是要显示的原因。
    model_block: Option<String>,
    /// `/usage` 刚算好的账单面板，等 daemon 取走发给发起命令的那个前端。
    ///
    /// 不在会话里长期保存：面板是**那一刻**的一张快照，用户关掉就没了，
    /// 下次 `/usage` 重新算（期间可能又跑了好几轮）。
    usage_panel: Option<(crate::server::wire::UsageLang, Vec<crate::server::store::ModelUsage>)>,
}

/// 按字符折 token：DeepSeek 的口径，英文 0.3 token/字符、中文 0.6。
///
/// 宽字符（CJK、假名、全角标点）算中文那一档——它们本来就是同一类 token 的
/// 粗粒度。这只是"还没实测过"时的占位，第一轮响应回来就被真值顶掉。
fn estimate_by_chars(text: &str) -> u64 {
    use unicode_width::UnicodeWidthChar;
    let mut narrow = 0u64;
    let mut wide = 0u64;
    for c in text.chars() {
        if UnicodeWidthChar::width(c).unwrap_or(0) >= 2 {
            wide += 1;
        } else {
            narrow += 1;
        }
    }
    ((narrow as f64) * 0.3 + (wide as f64) * 0.6).round() as u64
}

impl Session {
    /// Assemble a session service + its event channel. `chat` starts as
    /// the system-prompt-only replica.
    // A constructor wiring session resources one-to-one; a params struct
    // would just mirror these fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: SessionState,
        client: Client,
        chat: ChatContext,
        max_tokens: u32,
        cost: crate::server::ai::config::Cost,
        context_window: u64,
        model_name: String,
        model_provider: String,
        currency: crate::server::ai::config::Currency,
        cwd: std::path::PathBuf,
        tool_filter: Option<Vec<String>>,
        tools: crate::server::ai::config::ToolsConfig,
        browser: crate::server::ai::config::BrowserConfig,
        db_path: Option<std::path::PathBuf>,
    ) -> (Self, std::sync::mpsc::Receiver<SessionEvent>) {
        let (tx, rx) = std::sync::mpsc::channel::<SessionEvent>();
        // 服务商是落盘事实（`turns.provider`），归状态持有；会话层只负责在
        // 装配与 `/switch` 时告诉它。
        let mut state = state;
        state.model_provider = model_provider;
        let s = Self {
            state,
            client: std::cell::RefCell::new(client),
            chat: Arc::new(Mutex::new(chat)),
            max_tokens,
            interrupt: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            workspace_root: cwd.clone(),
            cwd: Arc::new(std::sync::RwLock::new(cwd)),
            cost,
            context_window,
            model_name,
            currency,
            cost_tracker: crate::server::ai::pricing::CostTracker::default(),
            tx,
            tool_filter,
            roster_source: None,
            tools,
            compact: crate::server::ai::config::CompactConfig::default(),
            commands: crate::server::commands::CommandEnv::default(),
            browser,
            artifact_db: db_path,
            stream_mode: crate::server::ai::config::StreamMode::Immediate,
            estimate: std::cell::Cell::new(None),
            model_block: None,
            usage_panel: None,
        };
        (s, rx)
    }

    /// `/compact [focus]`: run context compaction on a background thread.
    ///
    /// The summarization round-trip is a blocking network call (the
    /// non-streaming `complete` — no per-character display needed), so it
    /// must not sit on the UI thread. The thread owns a cloned `Client`
    /// (the real one stays behind the RefCell); results come back through
    /// the event channel:
    ///
    /// - `Error` echoes the failure (context untouched; safe to retry),
    /// - `Compaction { entries, ctx }` lets the session apply the fork
    ///   atomically: persist the marker entry, swap the live context,
    ///   echo the divider.
    ///
    /// Returns false when a turn is already streaming (compaction shares
    /// the busy gate — two writers on one context is a torn read).
    pub fn run_compact(&mut self, focus: &str, ccfg: &crate::server::ai::config::CompactConfig) -> bool {
        if self.state.busy() {
            return false;
        }
        let client = self.client.borrow().clone();
        let chat = self.chat.clone();
        let tx = self.tx.clone();
        let cwd_snapshot = self.state.transcript().to_vec();
        let system = chat
            .lock()
            .expect("chat 锁中毒")
            .messages
            .first()
            .map(|m| match m {
                crate::server::ai::types::Message::System { content } => content.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();
        let ccfg = ccfg.clone();
        let focus = focus.to_string();
        let max_tokens = self.max_tokens;
        std::thread::spawn(move || {
            let mut summarize = |req: &crate::server::ai::types::Context| -> anyhow::Result<String> {
                let reply = client.complete(req, max_tokens)?;
                Ok(reply.content)
            };
            match crate::server::compaction::compact(
                &system,
                &cwd_snapshot,
                &ccfg,
                &focus,
                &mut summarize,
            ) {
                Ok(out) => {
                    let _ = tx.send(SessionEvent::Compaction {
                        entries: vec![out.marker],
                        ctx: out.ctx,
                        tokens_before: out.tokens_before,
                        tokens_after: out.tokens_after,
                    });
                }
                Err(e) => {
                    let _ = tx.send(SessionEvent::Error(format!("{e:#}")));
                }
            }
        });
        true
    }

    /// Submit user text: lazily create the session (in-memory mode
    /// degrades gracefully), echo + stage + reset streaming slots, then
    /// spawn the turn runner. Returns false when a turn is already
    /// streaming.
    pub fn submit(&mut self, text: &str) -> bool {
        if self.state.busy() {
            return false;
        }
        if self.state.session_id().is_none() {
            let cwd = self.cwd().display().to_string();
            let Some(st) = self.state.store_mut() else {
                return self.finish_submit(text);
            };
            {
                let now = crate::server::store::now_stamp();
                match st.create_session(&now, &cwd) {
                    Ok(id) => {
                        self.state.adopt_session(id, Vec::new(), None);
                    }
                    Err(e) => {
                        self.state.echo(crate::server::entry::Entry::Error {
                            text: format!("数据库不可用：{e:#}"),
                        });
                    }
                }
            }
        }
        self.ensure_full_history();
        self.finish_submit(text)
    }

    /// 惰性历史补齐：第一回合之前必须把**完整**转录读回来。
    ///
    /// resume 只装了尾巴（第一帧才快），而模型上下文必须包含全部历史——
    /// 只发尾巴会让模型对着半截对话说话，而且不报错。所以这里补齐并重建
    /// 上下文副本；前端那边会收到一次整体快照（代变了）。
    fn ensure_full_history(&mut self) {
        if !self.state.history_pending() {
            return;
        }
        let Some((st, sid)) = self.state.persistence() else {
            self.state.set_history_pending(false);
            return;
        };
        match st.load_entries(sid) {
            Ok(all) => {
                let system = {
                    let chat = self.chat.lock().expect("chat 锁中毒");
                    match chat.messages.first() {
                        Some(crate::server::ai::types::Message::System { content }) => {
                            content.clone()
                        }
                        _ => crate::server::profile::BUILTIN_SYSTEM.to_string(),
                    }
                };
                *self.chat.lock().expect("chat 锁中毒") =
                    crate::server::turn::entries_to_context(&system, &all);
                self.state.replace_transcript(all);
            }
            Err(e) => {
                self.state.echo(crate::server::entry::Entry::Error {
                    text: format!("历史补齐失败：{e:#}"),
                });
            }
        }
        self.state.set_history_pending(false);
    }

    fn finish_submit(&mut self, text: &str) -> bool {
        self.state.start_turn(text);
        self.interrupt
            .store(false, std::sync::atomic::Ordering::Relaxed);
        crate::server::turn::spawn_turn(
            self.tx.clone(),
            crate::server::turn::TurnRequest {
                // 会话 id 在客户端上只在宿主声明了 `promptCacheSessionHeader`
                // 时才有用——它让宿主把同一会话粘到同一个后端实例，命中那边的
                // KV 缓存。每次开回合都重新喂一遍，是因为 client 是可以被
                // `switch_model` 换掉的。
                client: {
                    let mut c = self.client.borrow().clone();
                    if let Some(id) = self.state.session_id() {
                        c.set_session_id(id.to_string());
                    }
                    c
                },
                chat: self.chat.clone(),
                text: text.to_string(),
                cfg: crate::server::agent::loop_rs::LoopConfig::new(self.max_tokens),
                stream_mode: self.stream_mode,
                interrupt: self.interrupt.clone(),
                cwd: self.cwd.read().expect("cwd 锁中毒").clone(),
                workspace_root: self.workspace_root.clone(),
                todo: self.state.current_todo(),
                cwd_slot: self.cwd.clone(),
                history: std::sync::Arc::new(self.state.transcript().to_vec()),
                cwd_trail: std::sync::Arc::new(
                    self.state
                        .persistence()
                        .and_then(|(st, sid)| st.cwd_trail(sid).ok())
                        .unwrap_or_default(),
                ),
                tool_filter: self.tool_filter.clone(),
                tools: self.tools.clone(),
                browser: self.browser.clone(),
                // Session-scoped artifact store: the turn thread gets its
                // own WAL-mode connection; a failed open degrades to None
                // (oversized output then stays verbatim in the context).
                artifacts: match (self.artifact_db.as_deref(), self.state.session_id()) {
                    (Some(path), Some(sid)) => {
                        crate::server::agent::artifacts::ArtifactStore::open(path, sid)
                    }
                    _ => None,
                },
            },
        );
        true
    }

    /// Run one slash command (the `ClientMsg::Command` path).
    ///
    /// The table decides *what* a command is; this decides what this session can
    /// do about it. Commands that need configuration the session does not hold
    /// (a model resolver, a profile root) say so plainly instead of pretending
    /// to have run.
    pub fn run_command(
        &mut self,
        spec: &crate::server::commands::CommandSpec,
        args: &str,
    ) -> anyhow::Result<()> {
        use crate::server::commands::{ArgKind, Scope};
        anyhow::ensure!(
            spec.scope == Scope::Session,
            "{} 是前端自己的命令",
            spec.name
        );
        match spec.name {
            "/name" => {
                let name = args.trim();
                anyhow::ensure!(!name.is_empty(), "用法：{} <名字>", spec.name);
                self.ingest(SessionEvent::NameMarker(name.to_string()));
                self.notice(&format!("会话已命名为「{name}」"));
            }
            "/cdp" | "/cd" => {
                let raw = args.trim();
                anyhow::ensure!(!raw.is_empty(), "用法：{} <目录>", spec.name);
                let path = self.resolve_dir(raw)?;
                let previous = self.set_cwd(path.clone());
                self.ingest(SessionEvent::SetCwd {
                    path: path.display().to_string(),
                });
                self.notice(&format!(
                    "工作目录：{} → {}",
                    previous.display(),
                    path.display()
                ));
            }
            "/compact" => {
                let focus = args.trim().to_string();
                let ccfg = self.compact.clone();
                let started = self.run_compact(&focus, &ccfg);
                        anyhow::ensure!(started, "压缩没能启动（可能已经在跑）");
            }
            "/switch" => {
                let id = args.trim();
                anyhow::ensure!(!id.is_empty(), "用法：{} <模型 id，如 vendor:model>", spec.name);
                let resolve = self
                    .commands
                    .resolve_model
                    .clone()
                    .ok_or_else(|| anyhow!("模型解析器没接（这一侧还没有配置）"))?;
                let sw = resolve(id).map_err(|e| anyhow!("{e}"))?;
                self.apply_model(&sw);
                // 落盘：这是**这个会话**在用什么模型，不是默认配置。resume 时
                // 按它换回去，不然接着聊还得再敲一遍。
                self.remember_model(&sw.provider, &sw.model_id);
                self.notice(&format!(
                    "Session Only Model Switched into: {}:{}/{}",
                    sw.name, sw.provider, sw.model_id
                ));
            }
            "/usage" => {
                let lang = crate::server::wire::UsageLang::parse(args).map_err(|e| anyhow!("{e}"))?;
                // 价格解析器要在借出 store 之前取到手（`persistence` 会把整个
                // `self` 可变借走）。
                let resolve = self.commands.resolve_model.clone();
                let fx = self.commands.fx.clone();
                let (st, sid) = self
                    .persistence()
                    .ok_or_else(|| anyhow!("这个会话还没落盘，没有账单可查"))?;
                let mut groups = st.usage_by_model(sid)?;
                if let Some(resolve) = resolve.as_ref() {
                    for g in &mut groups {
                        // 认服务商 + 裸 id 才是完整地址（同名模型可以挂两家）；
                        // 认不出就留着 id 显示，金额那一栏说"没有价格表"。
                        let Some(provider) = g.provider.as_deref() else {
                            continue;
                        };
                        let Ok(sw) = resolve(&format!("{provider}:{}", g.id)) else {
                            continue;
                        };
                        let b = crate::server::ai::pricing::breakdown(
                            g.prompt_tokens,
                            g.cached_tokens,
                            g.completion_tokens,
                            &sw.cost,
                        );
                        // 一个会话换过模型就可能横跨两种币种，一张表里的数字
                        // 得同单位才可比：这里按汇率折成人民币（符号随第一栏）。
                        let (input, symbol) = fx.to_display(b.input, sw.currency);
                        let (output, _) = fx.to_display(b.output, sw.currency);
                        let (cache_read, _) = fx.to_display(b.cache_read, sw.currency);
                        let (cache_write, _) = fx.to_display(b.cache_write, sw.currency);
                        g.name = sw.name;
                        g.currency = Some(symbol.to_string());
                        g.cost = Some(crate::server::store::UsageCost {
                            input,
                            output,
                            cache_read,
                            cache_write,
                        });
                    }
                }
                // 面板不是转录条目：不进历史、不落盘、不参与上下文。它存在
                // 会话上，由 daemon 取走发给**敲命令的那个前端**。
                self.usage_panel = Some((lang, groups));
            }
            "/profile" => {
                let name = args.trim();
                anyhow::ensure!(!name.is_empty(), "用法：{} <profile 名>", spec.name);
                let resolve = self
                    .commands
                    .resolve_profile
                    .clone()
                    .ok_or_else(|| anyhow!("profile 解析器没接（这一侧还没有配置）"))?;
                let (system, roster) = resolve(name).map_err(|e| anyhow!("{e}"))?;
                self.apply_profile(&system, roster.clone());
                self.notice(&format!(
                    "profile 切到「{name}」（系统提示词已换；工具名册：{}）",
                    match &roster {
                        Some(r) => format!("{} 个", r.len()),
                        None => "全部".to_string(),
                    }
                ));
            }
            "/model" => {
                let id = args.trim();
                anyhow::ensure!(!id.is_empty(), "用法：{} <模型 id>", spec.name);
                let resolve = self
                    .commands
                    .resolve_model
                    .clone()
                    .ok_or_else(|| anyhow!("模型解析器没接（这一侧还没有配置）"))?;
                let write = self
                    .commands
                    .set_default_model
                    .clone()
                    .ok_or_else(|| anyhow!("配置写入器没接（这一侧还没有配置）"))?;
                // 解析在前：id 不存在就直接拒，绝不写坏用户的下一份配置。
                let sw = resolve(id).map_err(|e| anyhow!("{e}"))?;
                // 落脚点一：本会话的元数据（立刻生效）。
                self.apply_model(&sw);
                // 落脚点二：配置文件里对应的键（下次启动生效）。
                let where_ = write(id).map_err(|e| anyhow!("{e}"))?;
                self.notice(&format!(
                    "模型切到 {}（{}），并已写为默认：{}",
                    sw.name, sw.model_id, where_
                ));
            }
            other => anyhow::bail!("{other} 还没接线（下一轮）"),
        }
        let _ = ArgKind::None; // 参数形状由前端做补全，服务端只用它的值
        Ok(())
    }

    /// Point this session at another model: the client's endpoint/key/id, plus
    /// the display metadata the status line and the token gauge read.
    ///
    /// All of it moves together — a switched client with a stale context window
    /// or price sheet is worse than not switching at all.
    fn apply_model(&mut self, sw: &crate::server::commands::ModelSwitch) {
        self.client
            .borrow_mut()
            .switch_model(&sw.base_url, &sw.api_key, &sw.model_id, sw.compat.as_ref());
        self.cost = sw.cost;
        self.context_window = sw.context_window;
        self.model_name = sw.name.clone();
        self.currency = sw.currency;
        self.state.model_provider = sw.provider.clone();
        self.max_tokens = sw.max_tokens;
        // 换到能用的模型了，卡住的原因消失。
        self.model_block = None;
    }

    /// 把"这个会话在用什么模型"写进库（`/switch` 的落盘那一步）。
    ///
    /// 写失败只说一声，不回滚内存里的切换——屏幕上已经换了，为一次落盘失败
    /// 把它退回去更让人困惑。代价是下次 resume 回到默认模型，那条提示正好
    /// 交代了这件事。
    fn remember_model(&mut self, provider: &str, model_id: &str) {
        let Some(sid) = self.state.session_id() else {
            return;
        };
        let failure = match self.state.store_mut() {
            Some(st) => st.set_session_model(sid, Some((provider, model_id))).err(),
            None => None,
        };
        if let Some(e) = failure {
            self.notice(&format!("会话模型没能落盘（下次 resume 会回到默认）：{e:#}"));
        }
    }

    /// resume 之后：会话行里记着上次 `/switch` 到的模型就换回去。
    ///
    /// 换不回去（模型从配置里删了、id 改了）就**卡住这个会话**，绝不静默降级：
    /// 上下文一长，用错的模型接着跑就是真金白银。用户 `/switch` 到别的模型
    /// 就能继续——这条错误本身必须一直看得见，所以它是 `Entry::Error` 而不是
    /// 一条说完就撤的通知。
    pub fn restore_model(&mut self) {
        let Some(sid) = self.state.session_id() else {
            return;
        };
        let Some((provider, model_id)) = self
            .state
            .store()
            .and_then(|st| st.session_model(sid).ok().flatten())
        else {
            return;
        };
        let Some(resolve) = self.commands.resolve_model.clone() else {
            return;
        };
        // 会话命令认的全局 id 是 `provider:model`；`provider/model` 只是给人看
        // 的写法（模型 id 自己可能带斜杠，所以只有冒号那种拼法不歧义）。
        match resolve(&format!("{provider}:{model_id}")) {
            Ok(sw) => self.apply_model(&sw),
            Err(e) => {
                let text = format!(
                    "Current model is not available, check your `models.yml`, \
                     or use `/switch` to change your model\n（{provider}/{model_id}：{e}）"
                );
                self.model_block = Some(text.clone());
                self.state.echo(Entry::Error { text });
            }
        }
    }

    /// resume 之后：把这本账**接上**——库里按轮记着 token，拿当前价格表重算
    /// 一遍，就是重启前花掉的那些钱。
    ///
    /// 不接的话，状态栏的金额每次重启都从零开始，看起来像那个组件坏了；花过的
    /// 钱是事实，不该因为进程重启就消失。
    ///
    /// 认不出价格表的模型（配置里删了/改了名）那部分算不出来，跳过：金额少算
    /// 一点，比拿一个编出来的折扣价冒充事实强。那种会话本来也会被卡住（见
    /// [`Self::restore_model`]）。
    pub fn restore_spend(&mut self) {
        let resolve = self.commands.resolve_model.clone();
        let Some((st, sid)) = self.persistence() else {
            return;
        };
        let Ok(groups) = st.usage_by_model(sid) else {
            return;
        };
        let mut total = 0.0;
        for g in &groups {
            let (Some(provider), Some(resolve)) = (g.provider.as_deref(), resolve.as_ref()) else {
                continue;
            };
            let Ok(sw) = resolve(&format!("{provider}:{}", g.id)) else {
                continue;
            };
            total += crate::server::ai::pricing::breakdown(
                g.prompt_tokens,
                g.cached_tokens,
                g.completion_tokens,
                &sw.cost,
            )
            .total();
        }
        self.cost_tracker.total = total;
    }

    /// 会话被卡住的原因（模型不可用）。`None` = 可以正常发消息。
    pub fn model_block(&self) -> Option<&str> {
        self.model_block.as_deref()
    }

    /// 取走刚算好的 `/usage` 账单（取一次就没了：面板是一次性的快照）。
    pub fn take_usage_panel(
        &mut self,
    ) -> Option<(crate::server::wire::UsageLang, Vec<crate::server::store::ModelUsage>)> {
        self.usage_panel.take()
    }

    /// 当前计价货币的符号（状态栏金额用）。
    pub fn currency_symbol(&self) -> &'static str {
        self.currency.symbol()
    }

    /// Swap the system prompt and the tool roster.
    ///
    /// The system prompt lives in the live chat replica (the next request is
    /// built from it), so the change takes effect on the next round. The
    /// provider's prefix cache goes cold once — which is why the roster and the
    /// prompt only ever change at a boundary or on an explicit user command.
    fn apply_profile(&mut self, system: &str, roster: Option<Vec<String>>) {
        {
            let mut chat = self.chat.lock().expect("chat 锁中毒");
            match chat.messages.first_mut() {
                Some(crate::server::ai::types::Message::System { content }) => {
                    *content = system.to_string();
                }
                _ => chat
                    .messages
                    .insert(0, crate::server::ai::types::Message::System {
                        content: system.to_string(),
                    }),
            }
        }
        self.tool_filter = roster;
    }

    /// A directory argument for `/cdp`: relative to the current cwd, `~`
    /// expanded, must exist.
    fn resolve_dir(&self, raw: &str) -> anyhow::Result<std::path::PathBuf> {
        let p = if let Some(rest) = raw.strip_prefix('~') {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
        } else {
            std::path::PathBuf::from(raw)
        };
        let p = if p.is_absolute() { p } else { self.cwd().join(p) };
        let real = p
            .canonicalize()
            .with_context(|| format!("没有这个目录：{raw}"))?;
        anyhow::ensure!(real.is_dir(), "不是目录：{raw}");
        Ok(real)
    }

    /// Append a system notice to the transcript (memory only; never persisted —
    /// see `Entry::System`).
    pub fn notice(&mut self, text: &str) {
        self.state.echo(Entry::System {
            text: text.to_string(),
            align: crate::server::entry::Align::Left,
            pin: false,
        });
    }

    /// Migrate the working directory; returns the previous value.
    pub fn set_cwd(&self, next: std::path::PathBuf) -> std::path::PathBuf {
        let mut w = self.cwd.write().expect("cwd 锁中毒");
        std::mem::replace(&mut *w, next)
    }

    pub fn cwd(&self) -> std::path::PathBuf {
        self.cwd.read().expect("cwd 锁中毒").clone()
    }

    /// 用量表的起点（算一次、缓存住）：第一轮响应回来之前，表上显示的就是它。
    ///
    /// 两种处境，取法不同：
    /// - **恢复了历史**（转录非空，而内存里还没有实测值——进程刚起来）：用库里
    ///   最后一条回复记的 `prompt_tokens`。那段对话早就不止开场白那么长了，
    ///   拿开场白的体重顶上去会指着一个明显偏小的数。
    /// - **还没跑过回合**：借同一份开场白在历史里的最小实测值（`min_prompt_tokens`），
    ///   一次都没记录过就按字符折（[`estimate_by_chars`]）。
    pub fn context_estimate(&self) -> u64 {
        if let Some(n) = self.estimate.get() {
            return n;
        }
        let n = self.compute_estimate();
        self.estimate.set(Some(n));
        n
    }

    fn compute_estimate(&self) -> u64 {
        // 恢复来的历史：库里最后一条回复记得这个上下文有多重。
        if self.cost_tracker.last_prompt_tokens == 0
            && !self.state.transcript().is_empty()
            && let Some(st) = self.state.store()
            && let Some(sid) = self.state.session_id()
            && let Ok(Some(n)) = st.last_prompt_tokens(sid)
        {
            return n;
        }
        let (system, tools_json) = {
            let Ok(chat) = self.chat.lock() else {
                return 0;
            };
            (
                crate::server::turn::system_message(&chat),
                serde_json::to_string(&chat.tools).unwrap_or_default(),
            )
        };
        if let Some(st) = self.state.store()
            && let Ok(Some(n)) = st.min_prompt_tokens(&system, &tools_json, self.client.borrow().model())
        {
            return n;
        }
        estimate_by_chars(&system) + estimate_by_chars(&tools_json)
    }

    /// Current model's display price: total spent + latest prompt tokens.
    pub fn spend(&self) -> (f64, u64) {
        (
            self.cost_tracker.total,
            self.cost_tracker.last_prompt_tokens,
        )
    }

    // ---- read-side passthrough (the surface renders through these) ----

    pub fn busy(&self) -> bool {
        self.state.busy()
    }
    pub fn stream_view(&self) -> &StreamView {
        self.state.stream_view()
    }
    pub fn transcript(&self) -> &[Entry] {
        self.state.transcript()
    }
    pub fn transcript_generation(&self) -> u64 {
        self.state.transcript_generation()
    }
    pub fn session_name(&self) -> Option<&str> {
        self.state.session_name()
    }
    pub fn session_id(&self) -> Option<i64> {
        self.state.session_id()
    }
    /// The model id this session talks to (status line + `state` message).
    pub fn model(&self) -> String {
        self.client.borrow().model().to_string()
    }
    /// Context window (models.yml) for the usage gauge's denominator.
    /// 0 = unknown — the gauge hides instead of showing a wrong ratio.
    pub fn context_window(&self) -> u64 {
        self.context_window
    }
    /// Display name for the statusline: models.yml `name` when set, else
    /// the raw id. Pure metadata; nothing behavioral reads it.
    /// The system prompt the **next** request will carry (messages[0]).
    ///
    /// `/profile` swaps it; a resume preserves it. Read-only so tests and the
    /// daemon can observe what the provider is about to be told.
    pub fn system_prompt(&self) -> String {
        match self.chat.lock().expect("chat 锁中毒").messages.first() {
            Some(crate::server::ai::types::Message::System { content }) => content.clone(),
            _ => String::new(),
        }
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }
    pub fn store(&self) -> Option<&Store> {
        self.state.store()
    }
    pub fn store_mut(&mut self) -> Option<&mut Store> {
        self.state.store_mut()
    }
    pub fn pending_snapshot(&self) -> &[Entry] {
        self.state.pending_snapshot()
    }

    // ---- write-side passthrough (UI-flow mutators that predate the
    // protocol: echoes, navigation, resume adoption). The turn's data
    // path itself goes through `handle` only. ----

    pub fn echo(&mut self, e: Entry) -> Change {
        self.state.echo(e)
    }
    pub fn stage(&mut self, e: Entry) {
        self.state.stage(e)
    }
    pub fn commit_round(&mut self, entries: Vec<Entry>) {
        self.state.commit_round(entries)
    }
    pub fn adopt_session(&mut self, id: i64, entries: Vec<Entry>, name: Option<String>) -> Change {
        self.state.adopt_session(id, entries, name)
    }
    pub fn retarget(&mut self, id: i64, entries: Vec<Entry>) {
        self.state.retarget(id, entries)
    }
    pub fn set_session_name(&mut self, name: Option<String>) {
        self.state.set_session_name(name)
    }
    pub fn ensure_session(&mut self, root: &std::path::Path) -> Option<i64> {
        self.state.ensure_session(root)
    }
    pub fn persistence(&mut self) -> Option<(&mut Store, i64)> {
        self.state.persistence()
    }

    /// 惰性历史：尾巴之后还有更老的条目（daemon 据此起后台补发）。
    pub fn history_pending(&self) -> bool {
        self.state.history_pending()
    }

    /// resume 时登记"只装了尾巴"（见 `hub::resume`）。
    pub fn set_history_pending(&mut self, pending: bool) {
        self.state.set_history_pending(pending);
    }

    /// 还没落盘的那截尾巴（daemon 随块增量一起发）。
    pub fn live(&self) -> &[Entry] {
        self.state.live()
    }

    /// 见 [`SessionState::blocks_clean`]。
    pub fn blocks_clean(&self) -> bool {
        self.state.blocks_clean()
    }

    /// 见 [`SessionState::reset_blocks_clean`]。
    pub fn reset_blocks_clean(&mut self) {
        self.state.reset_blocks_clean();
    }
    pub fn handle(&mut self, ev: SessionEvent) -> Change {
        self.state.handle(ev)
    }

    /// Rebuild one stored round's request from the database alone (see
    /// [`SessionState::replay_round`]).
    pub fn replay_round(&self, session_id: i64, round_seq: i64) -> Result<Replay, String> {
        self.state.replay_round(session_id, round_seq)
    }

    /// 分叉：抛下在跑的回合，把这条会话接上新分支。
    ///
    /// 三件事缺一不可，顺序也有讲究：
    ///
    /// 1. **打断**（flag + 杀 worker）：旧回合的读循环立刻结束，不再烧 token。
    /// 2. **换事件通道**：旧回合线程还握着旧 `tx`，它剩下的收尾事件（TurnDone
    ///    / Error / Done）必须落进一个没人读的信道——否则它们会以**新分支**
    ///    的 id 落盘（`persistence()` 读的是当前 id）。
    /// 3. **换上下文副本**：旧线程握着旧 `Arc`，结束时写回的是**那一个**。不换
    ///    的话，这里按新前缀重建的上下文会被被抛弃分支的消息覆盖。
    ///
    /// 最后复位流式/回合状态：`busy()` 读的就是 `stream.active`，不清的话新
    /// 分支一开工就被"正在回复中"挡回去。
    ///
    /// 调用方（hub）**先** `retarget` 再调这里——上下文要按新前缀重建。
    /// 返回新的接收端，由调用方持有。
    pub fn abandon_turn(&mut self) -> std::sync::mpsc::Receiver<SessionEvent> {
        self.interrupt
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.client.borrow().interrupt_worker();
        let (tx, rx) = std::sync::mpsc::channel();
        self.tx = tx;
        let system = {
            let chat = self.chat.lock().expect("chat 锁中毒");
            match chat.messages.first() {
                Some(crate::server::ai::types::Message::System { content }) => content.clone(),
                _ => crate::server::profile::BUILTIN_SYSTEM.to_string(),
            }
        };
        let ctx = crate::server::turn::entries_to_context(&system, self.state.transcript());
        self.chat = std::sync::Arc::new(std::sync::Mutex::new(ctx));
        self.state.abandon_turn();
        rx
    }

    /// Esc during a turn: the runner stops reading and disconnects.
    pub fn interrupt_turn(&self) {
        self.interrupt.store(true, std::sync::atomic::Ordering::Relaxed);
        self.client.borrow().interrupt_worker();
    }

    /// Drain protocol events into the session state. `Change::TurnDone`
    /// is priced locally against `cost` here. Returns the observed
    /// changes so the surface reacts without touching internals.
    pub fn ingest(&mut self, ev: SessionEvent) -> Change {
        // Compaction lands at the facade: it must touch the chat replica
        // (owned here, not by SessionState) and the store in one go.
        if let SessionEvent::Compaction {
            entries,
            ctx,
            tokens_before,
            tokens_after,
        } = ev
        {
            return self.apply_compaction(entries, ctx, tokens_before, tokens_after);
        }
        let change = self.state.handle(ev);
        if change == Change::TurnDone
            && let Some(u) = self.state.take_last_usage()
        {
            self.cost_tracker.record(&u, &self.cost);
        }
        change
    }

    /// Attach what commands may reach outside the session. Called once, at
    /// assembly, from the spec.
    pub fn with_commands(mut self, env: crate::server::commands::CommandEnv) -> Self {
        self.commands = env;
        self
    }

    /// Attach the compaction knobs (`/compact` needs them). Called once, at
    /// assembly, from the spec.
    pub fn with_compact(mut self, cfg: crate::server::ai::config::CompactConfig) -> Self {
        self.compact = cfg;
        self
    }

    /// Let the roster be re-read at a legal boundary (see
    /// `tools.reloadOnCompaction`). Called once, at assembly.
    pub fn with_roster_source(
        mut self,
        source: std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync>,
    ) -> Self {
        self.roster_source = Some(source);
        self
    }

    /// The roster the model is being offered right now (profile-filtered).
    /// `None` = no profile restriction.
    /// The `/compact` knobs this session was assembled with.
    pub fn compact_knobs(&self) -> &crate::server::ai::config::CompactConfig {
        &self.compact
    }

    pub fn tool_roster(&self) -> Option<&Vec<String>> {
        self.tool_filter.as_ref()
    }

    /// Atomically apply a finished compaction: persist the fork marker,
    /// swap the live context, echo the divider + stats.
    fn apply_compaction(
        &mut self,
        entries: Vec<Entry>,
        ctx: ChatContext,
        tokens_before: usize,
        tokens_after: usize,
    ) -> Change {
        // 1) Persist the marker under the current leaf (the tree keeps
        //    the pre-compact branch reachable).
        let stored = match self.state.persistence() {
            Some((st, sid)) => match st.append(sid, &entries) {
                Ok(_) => true,
                Err(e) => {
                    let msg = format!("压缩标记落盘失败：{e:#}");
                    self.state.echo(Entry::Error { text: msg });
                    false
                }
            },
            None => false,
        };
        // 2) Swap the live context replica: next turn starts from
        //    system + summary turn + kept region (prefix-cache cold
        //    once, then warm).
        *self.chat.lock().expect("chat 锁中毒") = ctx;
        // 3) Transcript: the divider marker + the display stats. 落盘成功的那份
        //    以"已落盘条目"上屏（daemon 随后按块 id 发，两边不会各发一条）。
        for e in entries {
            if stored {
                self.state.push_stored(e);
            } else {
                self.state.echo(e);
            }
        }
        self.state.echo(Entry::System {
            text: format!("上下文已压缩：≈{tokens_before} → ≈{tokens_after} tokens"),
            align: crate::server::entry::Align::Center,
            pin: false,
        });
        // 4) A compaction rewrites the conversation anyway, so it is the one
        //    other legal seam for the roster to change (`tools.reloadOnCompaction`).
        //    Off by default: a tool that appears mid-session leaves the model
        //    holding results from a tool it can no longer see.
        if self.tools.reload_on_compaction
            && let Some(source) = &self.roster_source
        {
            self.tool_filter = source();
        }
        Change::Session
    }

    /// Coarse run state — what a client that renders no transcript needs in
    /// order to say "思考中" / "正在调用工具" without parsing deltas or tool
    /// payloads. Derived from the streaming slots, so it cannot drift from what
    /// the rich clients show.
    pub fn status(&self) -> RunState {
        self.state.stream_view().run_state()
    }

    /// Switch stream delivery mode. Takes effect at the next round, so one
    /// round's output is never split across two modes.
    pub fn set_stream_mode(&mut self, mode: crate::server::ai::config::StreamMode) {
        self.stream_mode = mode;
    }

    pub fn stream_mode(&self) -> crate::server::ai::config::StreamMode {
        self.stream_mode
    }

    pub fn drain(&mut self, rx: &std::sync::mpsc::Receiver<SessionEvent>) -> Vec<Change> {
        let mut changes = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            changes.push(self.ingest(ev));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> SessionState {
        SessionState::new(None) // in-memory mode
    }

    #[test]
    fn the_todo_memo_survives_a_turn_that_died_before_its_end() {
        // `Entry::Todo` 只在回合结束时落；卡片（工具结果）当场就落。回合中途
        // 死掉时，备忘录必须还能从结果里捞回来——否则模型"失忆"。
        use crate::server::entry::{TodoPhase, TodoStatus, TodoTask};
        let mut s = SessionState::new(None);
        let phases = vec![TodoPhase {
            name: "阶段一".into(),
            tasks: vec![TodoTask {
                content: "甲".into(),
                status: TodoStatus::Done,
                blocker: None,
            }],
        }];
        // 只有工具结果，没有 Entry::Todo（回合没走到头）。
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "todo".into(),
            ok: true,
            result: "[x] 甲".into(),
            details: Some(serde_json::json!({
                "kind": "todo",
                "op": "done",
                "phases": phases,
                "done": 1,
                "total": 1,
            })),
            duration_ms: 1,
        });
        let recovered = s.current_todo();
        assert_eq!(recovered.len(), 1, "从工具结果的 details 里捞回来");
        assert_eq!(recovered[0].tasks[0].status, TodoStatus::Done);
    }

    /// 顺序：**思考 → 工具 → 思考 → 正文**。
    ///
    /// 回合里的思考缓冲曾经只在 `finalize_round` 折成条目，于是"边想边调
    /// 工具"的一轮在屏幕上变成"工具 → 工具 → 思考 → 正文"，时间线整个倒了。
    #[test]
    fn reasoning_lands_before_the_tool_call_that_followed_it() {
        let mut s = SessionState::new(None);
        s.start_turn("干活");
        let _ = s.handle(SessionEvent::ReasoningDelta("先想".into()));
        let _ = s.handle(SessionEvent::ToolStart {
            call_id: "c1".into(),
            name: "read".into(),
            args: "{}".into(),
            intent: "读".into(),
            text: String::new(),
            first: true,
        });
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "read".into(),
            ok: true,
            result: "内容".into(),
            details: None,
            duration_ms: 1,
        });
        // 工具回来后接着想，再给正文。
        let _ = s.handle(SessionEvent::ReasoningDelta("再想".into()));
        let _ = s.handle(SessionEvent::Delta("正文".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));

        let kinds: Vec<&str> = s
            .transcript()
            .iter()
            .map(|e| match e {
                Entry::Reasoning { .. } => "思考",
                Entry::ToolRequest { .. } => "工具调用",
                Entry::ToolResult { .. } => "工具结果",
                Entry::Assistant { .. } => "正文",
                _ => "其他",
            })
            .collect();
        let kept: Vec<&&str> = kinds
            .iter()
            .filter(|k| **k != "其他")
            .collect();
        assert_eq!(
            kept,
            vec![&"思考", &"工具调用", &"工具结果", &"思考", &"正文"],
            "时间顺序：{:?}",
            kinds
        );
    }

    #[test]
    fn a_running_tools_output_lands_in_the_live_slot_and_stays_bounded() {
        // The live slot is a *display* buffer, not a transcript: it holds what
        // a running tool has printed, bounded to the tail (the finished result
        // carries everything anyway), and it goes away when the call ends.
        let mut s = SessionState::new(None);
        let _ = s.handle(SessionEvent::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: "跑".into(),
            text: String::new(),
            first: true,
        });
        let _ = s.handle(SessionEvent::ToolProgress {
            call_id: "c1".into(),
            chunk: "第一行\n第二行\n".into(),
        });
        assert_eq!(s.stream_view().tool_output, "第一行\n第二行\n");

        // A chatty command cannot grow the buffer without bound.
        let _ = s.handle(SessionEvent::ToolProgress {
            call_id: "c1".into(),
            chunk: "x".repeat(40 * 1024),
        });
        let out = &s.stream_view().tool_output;
        assert!(out.len() <= TOOL_OUTPUT_TAIL + 8, "缓冲区必须有界: {}", out.len());
        assert!(out.starts_with("…\n"), "截断要有标记");
        assert!(out.ends_with('x'), "留的是尾巴（最近打印的才算数）");

        // The result entry carries the real thing, so the live slot is cleared.
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: true,
            result: "done".into(),
            details: None,
            duration_ms: 1,
        });
        assert!(s.stream_view().tool_output.is_empty());
    }

    #[test]
    fn the_coarse_state_never_needs_the_payload() {
        // 受限客户端（电报那种）只看这一个值：思考 / 出话 / 调工具 / 空闲。
        use crate::server::events::RunState;
        let mut s = st();
        assert_eq!(s.stream_view().run_state(), RunState::Idle);
        let _ = s.start_turn("问");
        assert_eq!(
            s.stream_view().run_state(),
            RunState::Thinking,
            "刚发出去、什么都没回来 = 在想"
        );
        let _ = s.handle(SessionEvent::ReasoningDelta("想".into()));
        assert_eq!(s.stream_view().run_state(), RunState::Thinking);
        let _ = s.handle(SessionEvent::Delta("说".into()));
        assert_eq!(s.stream_view().run_state(), RunState::Replying);
        // 真实路径里，工具事件会带上它所属那条消息的正文（`text` 骑在首个调用上），
        // 于是一直挂着的正文槽在这里被清掉。
        let _ = s.handle(SessionEvent::ToolStart {
            call_id: "c".into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: "跑个命令".into(),
            text: "说".into(),
            first: true,
        });
        assert_eq!(
            s.stream_view().run_state(),
            RunState::Tool {
                intent: "跑个命令".into()
            }
        );
        assert!(
            s.stream_view().text.is_empty(),
            "那半句已经随工具条目存档，流式槽位不该再留着它——留着就是屏幕上两份"
        );
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c".into(),
            name: "bash".into(),
            ok: true,
            result: "ok".into(),
            details: None,
            duration_ms: 0,
        });
        assert_eq!(
            s.stream_view().run_state(),
            RunState::Thinking,
            "工具跑完还没轮到模型说话"
        );
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        assert_eq!(s.stream_view().run_state(), RunState::Idle);
    }

    // ---- 请求头落盘 + 逐字节复现 ----

    // 驱动一轮真实事件流：`start_turn` 才是用户消息的入口（`Submit` 是给
    // 「谁去开线程」用的，状态机对它不做事）。
    fn round(user: &str, req: &[SessionEvent], dir: &std::path::Path) -> (SessionState, i64) {
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(dir).unwrap();
        let _ = s.start_turn(user);
        for ev in req {
            let _ = s.handle(ev.clone());
        }
        (s, id)
    }

    fn header(model: &str, system: &str) -> SessionEvent {
        SessionEvent::RequestMeta {
            model: model.into(),
            protocol: "openai-chat-completions".into(),
            base_url: "https://api.test/v1".into(),
            system: system.into(),
            tools_json: r#"[{"type":"function","function":{"name":"bash"}}]"#.into(),
            max_tokens: 4096,
            compat_json: None,
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mypi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_round_stores_its_request_header_and_stop_reason() {
        // 一轮结束，库里除了转录还必须有一条「这次请求长什么样」：模型、端点、
        // 系统提示词、工具手册、token 上限、结束原因、以及它写了哪几条。
        let dir = tmp("reqmeta");
        let (s, id) = round(
            "问",
            &[
                SessionEvent::RequestMeta {
                    model: "vendor/model-x".into(),
                    protocol: "openai-chat-completions".into(),
                    base_url: "https://api.test/v1".into(),
                    system: "你是助手".into(),
                    tools_json: r#"[{"type":"function"}]"#.into(),
                    max_tokens: 1234,
                    compat_json: None,
                },
                SessionEvent::Delta("答".into()),
                SessionEvent::TurnDone(Usage::default(), crate::server::ai::types::StopReason::Stop),
            ],
            &dir,
        );
        let rows = s.store().unwrap().rounds(id).unwrap();
        assert_eq!(rows.len(), 1, "一轮 = 一条请求头");
        let r = &rows[0];
        assert_eq!(r.model, "vendor/model-x");
        assert_eq!(r.base_url, "https://api.test/v1");
        assert_eq!(r.system, "你是助手");
        assert_eq!(r.tools_json, r#"[{"type":"function"}]"#);
        assert_eq!(r.max_tokens, 1234);
        assert_eq!(r.stop_reason.as_deref(), Some("stop"));
        assert_eq!(r.first_block, Some(1));
        assert_eq!(r.last_block, Some(2), "user + assistant 两条");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_rebuilds_the_request_from_the_database_alone() {
        // 复现只准读库：系统提示词、工具手册、模型、端点全部来自那一行。
        // 这里没有 client、没有 config、没有 profile，能重建出来才算数。
        let dir = tmp("replay");
        let (s, id) = round(
            "你好",
            &[
                header("m", "系统提示词原文"),
                SessionEvent::Delta("你好呀".into()),
                SessionEvent::TurnDone(Usage::default(), crate::server::ai::types::StopReason::Stop),
            ],
            &dir,
        );
        let rp = s
            .replay_round(id, 1)
            .expect("从库里必须能复现这一轮的请求");
        assert_eq!(rp.model, "m");
        assert_eq!(rp.base_url, "https://api.test/v1");
        assert_eq!(rp.tools_json, r#"[{"type":"function","function":{"name":"bash"}}]"#);
        // body 里的东西全部来自库
        assert_eq!(rp.body["model"], serde_json::json!("m"));
        assert_eq!(rp.body["stream"], serde_json::json!(true));
        assert_eq!(rp.body["messages"][0]["role"], serde_json::json!("system"));
        assert_eq!(
            rp.body["messages"][0]["content"],
            serde_json::json!("系统提示词原文")
        );
        assert_eq!(rp.body["messages"][1]["content"], serde_json::json!("你好"));
        assert_eq!(rp.body["messages"][2]["content"], serde_json::json!("你好呀"));
        assert_eq!(rp.body["tools"][0]["function"]["name"], serde_json::json!("bash"));
        assert!(rp.body["max_tokens"].as_u64().unwrap() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_round_that_died_mid_flight_is_still_stored_complete() {
        // 网络中断：没有 TurnDone，只有 Error。已流出的正文要落盘，请求头也要，
        // 而 stop_reason 留空 —— 那是「网关没回答」的诚实记录。
        let dir = tmp("died");
        let (s, id) = round(
            "问",
            &[
                header("m", "S"),
                SessionEvent::Delta("半截回".into()),
                SessionEvent::Error("stream read interrupted".into()),
            ],
            &dir,
        );
        let rows = s.store().unwrap().rounds(id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].stop_reason, None, "没收到停因就必须是 NULL");
        let entries = s.store().unwrap().load_entries(id).unwrap();
        assert_eq!(
            entries,
            vec![
                Entry::User {
                    content: "问".into()
                },
                Entry::Assistant {
                    content: "半截回".into(),
                    usage: None
                },
            ],
            "卡到最后一个字也要落盘"
        );
        // 而且能复现
        let rp = s.replay_round(id, 1).expect("半途死掉的回合同样可复现");
        assert_eq!(rp.body["messages"][1]["content"], serde_json::json!("问"));
        assert_eq!(rp.body["messages"][2]["content"], serde_json::json!("半截回"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_interrupted_round_records_the_interrupt() {
        // 用户按 Esc：内容照库，但「被打断」这件事必须能从库里读出来，
        // 否则被打断的回合和正常结束的回合在磁盘上一样。
        let dir = tmp("interrupt");
        let (s, id) = round(
            "问",
            &[
                header("m", "S"),
                SessionEvent::Delta("被打断的".into()),
                SessionEvent::TurnDone(
                    Usage::default(),
                    crate::server::ai::types::StopReason::Interrupted,
                ),
            ],
            &dir,
        );
        let rows = s.store().unwrap().rounds(id).unwrap();
        assert_eq!(rows[0].stop_reason.as_deref(), Some("interrupted"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_round_replays_as_of_its_own_tip() {
        // 复现一个老回合，必须按它当时那一块为界重建，而不是按今天的分支末端
        // ——否则多出来的后续回合会把请求字节改掉。
        let dir = tmp("rewind");
        let (mut s, id) = round(
            "第一问",
            &[
                header("m1", "S1"),
                SessionEvent::Delta("第一答".into()),
                SessionEvent::TurnDone(Usage::default(), crate::server::ai::types::StopReason::Stop),
            ],
            &dir,
        );
        let _ = s.start_turn("第二问");
        let _ = s.handle(header("m2", "S2"));
        let _ = s.handle(SessionEvent::Delta("第二答".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let all = s.store().unwrap().load_entries(id).unwrap();
        assert_eq!(all.len(), 4, "两问两答都在库里");
        let rp = s.replay_round(id, 1).unwrap();
        assert_eq!(rp.model, "m1");
        assert_eq!(
            rp.messages.as_array().unwrap().len(),
            3,
            "第一轮那条链：system + 一问 + 一答"
        );
        assert_eq!(rp.messages[2]["content"], serde_json::json!("第一答"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_refuses_a_round_with_no_stored_header() {
        // 没有请求头 = 复现不出来。宁可报错，也不许给一份「看着挺像」的请求。
        let dir = tmp("noheader");
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("答".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        assert_eq!(s.store().unwrap().rounds(id).unwrap().len(), 0);
        let err = s.replay_round(id, 1).unwrap_err();
        assert!(err.contains("请求头"), "错误必须点名缺什么：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_text_round_persists_what_streamed() {
        // Drive the real event flow end to end: user submit, deltas, done.
        // The round that lands in the store must be exactly the events the
        // session consumed — no second assembler exists to disagree.
        let dir = std::env::temp_dir().join(format!("mypi-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        assert_eq!(
            back,
            vec![
                Entry::User {
                    content: "问".into()
                },
                Entry::Assistant {
                    content: "答案".into(),
                    usage: Some(Entry::usage_summary(&Usage::default())),
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_round_persists_requests_results_and_reply_with_reasoning() {
        // A tool round driven through events: the reasoning buffer folds in
        // right before the reply, the tool pair keeps its real `ok`, and the
        // round lands in one transaction.
        let dir = std::env::temp_dir().join(format!("mypi-toolround-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("跑个命令");
        let _ = s.handle(SessionEvent::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: "跑".into(),
            text: String::new(),
            first: true,
        });
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: false,
            result: "boom".into(),
            details: None,
            duration_ms: 0,
        });
        let _ = s.handle(SessionEvent::ReasoningDelta("查一下".into()));
        let _ = s.handle(SessionEvent::Delta("搞定".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                Entry::ToolRequest { .. } => "tool_request",
                Entry::ToolResult { .. } => "tool_result",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "user",
                "tool_request",
                "tool_result",
                "reasoning",
                "assistant"
            ],
            "思考必须紧邻最终回复、在工具条目之后"
        );
        // The **real** ok survives: a failed tool stays failed on disk.
        let ok = back.iter().find_map(|e| match e {
            Entry::ToolResult { ok, .. } => Some(*ok),
            _ => None,
        });
        assert_eq!(ok, Some(false), "失败的 ok 必须落盘（旧路径硬编码 true）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reasoning_and_usage_land_on_the_persisted_round() {
        let dir = std::env::temp_dir().join(format!("mypi-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::ReasoningDelta("想了想".into()));
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["user", "reasoning", "assistant"]);
        // Usage rode on the reply (the wire Message cannot carry it).
        let usage = back
            .iter()
            .find_map(|e| match e {
                Entry::Assistant { usage, .. } => Some(*usage),
                _ => None,
            })
            .unwrap();
        assert!(usage.is_some(), "usage 必须写在回复条目上");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reasoning_survives_into_the_assistant_entry() {
        let mut s = st();
        let _ = s.handle(SessionEvent::ReasoningDelta("先想".into()));
        let _ = s.handle(SessionEvent::ReasoningDelta("再想".into()));
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let ts = s.transcript();
        let reasoning = ts.iter().find_map(|e| match e {
            Entry::Reasoning { content } => Some(content.clone()),
            _ => None,
        });
        let reply = ts
            .iter()
            .find_map(|e| match e {
                Entry::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .expect("必须有一条 Assistant");
        assert_eq!(reply, "答案");
        assert_eq!(
            reasoning.as_deref(),
            Some("先想再想"),
            "reasoning 必须落成独立条目"
        );
    }

    #[test]
    fn a_stream_death_still_persists_the_partial_reply() {
        // The hard constraint: a dropped connection must not cost the user
        // the reply they already watched arrive. Error while streaming ->
        // whatever was produced is finalized and stored, then the error is
        // recorded (display-only). Replayable from any break point.
        let dir = std::env::temp_dir().join(format!("mypi-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("半句".into()));
        let _ = s.handle(SessionEvent::ReasoningDelta("想着".into()));
        // Connection dies mid-stream.
        let _ = s.handle(SessionEvent::Error("stream read interrupted".into()));

        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["user", "reasoning", "assistant"],
            "断网也要落盘已到达的部分"
        );
        let reply = back.iter().find_map(|e| match e {
            Entry::Assistant { content, .. } => Some(content.clone()),
            _ => None,
        });
        assert_eq!(reply.as_deref(), Some("半句"), "部分回复必须保留");
        // The error itself is display-only — not one of the stored kinds.
        assert!(
            back.iter().all(|e| !matches!(e, Entry::Error { .. })),
            "错误提示不落盘（无协议角色）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_finish_signals_tool_activity() {
        // The git-refresh contract: ToolFinish is the checkpoint surfaces
        // refresh the environment on — NOT plain transcript changes.
        let mut s = st();
        let ch = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: true,
            result: "ok".into(),
            details: None,
            duration_ms: 0,
        });
        assert_eq!(ch, Change::ToolActivity);
        // Speech-side changes stay Transcript.
        assert_eq!(
            s.handle(SessionEvent::Error("x".into())),
            Change::Transcript
        );
    }

    /// 没有历史可借时的字符折算：DeepSeek 的口径，英文 0.3 token/字符、
    /// 中文 0.6，宽字符（CJK、全角）算中文那一档。
    #[test]
    fn the_char_estimate_follows_the_ds_ratio() {
        assert_eq!(estimate_by_chars(""), 0);
        assert_eq!(estimate_by_chars("abcdefghij"), 3, "10 个英文 → 3");
        assert_eq!(estimate_by_chars("十个中文字符测试一下"), 6, "10 个中文 → 6");
        // 混排各算各的：3 个英文 + 2 个中文 = 0.9 + 1.2 = 2.1 → 2
        assert_eq!(estimate_by_chars("abc中文"), 2);
    }

    #[test]
    fn echo_grows_transcript() {
        let mut s = st();
        assert_eq!(
            s.echo(Entry::Error { text: "x".into() }),
            Change::Transcript
        );
        assert_eq!(s.transcript().len(), 1);
    }

    /// 短命的系统通知：块一落盘就撤。`/switch` 的告示说的是"刚刚切了模型"，
    /// 下一轮都写库了它还挂在屏幕上，那就只是过期的告示——而且它从不落盘，
    /// 恢复历史时也不会回来，留着只有这一刻的屏幕看得见。
    #[test]
    fn an_unpinned_notice_dies_once_a_round_lands() {
        let mut s = st();
        let _ = s.echo(Entry::System {
            text: "本会话模型切到 X".into(),
            align: crate::server::entry::Align::Left,
            pin: false,
        });
        assert_eq!(s.live().len(), 1);

        // 真实顺序：告示在前，这一轮的用户消息在后（它才是要落盘的那条）。
        let asked = Entry::User {
            content: "下一问".into(),
        };
        let _ = s.push_live(asked.clone());
        assert_eq!(s.live().len(), 2);

        // 撤掉通知**不该**把顺序判成脏：留下的序列仍是干净的。
        assert!(s.note_persisted(&[asked]), "撤通知不算顺序脏");
        assert!(s.live().is_empty(), "落盘后告示该撤：{:?}", s.live());
    }

    /// 钉住的不算短命：`pin` 的语义就是"常住"。
    #[test]
    fn a_pinned_notice_outlives_the_round_it_preceded() {
        let mut s = st();
        let _ = s.echo(Entry::System {
            text: "常住告示".into(),
            align: crate::server::entry::Align::Left,
            pin: true,
        });
        let landed = vec![Entry::User {
            content: "问".into(),
        }];
        s.note_persisted(&landed);
        assert_eq!(s.live().len(), 1, "钉住的留下");
    }

    #[test]
    fn commit_round_clears_pending() {
        let mut s = st();
        s.stage(Entry::User {
            content: "hi".into(),
        });
        assert_eq!(s.pending_snapshot().len(), 1);
        s.commit_round(vec![Entry::User {
            content: "hi".into(),
        }]);
        assert_eq!(s.pending_snapshot().len(), 0);
        assert_eq!(s.transcript().len(), 1);
    }

    #[test]
    fn name_marker_updates_statusline_and_appends_marker() {
        let mut s = st();
        s.handle(SessionEvent::NameMarker("test".into()));
        assert_eq!(s.session_name(), Some("test"));
        // Marker echoes into the transcript (renderer skips it).
        assert!(matches!(s.transcript().last(), Some(Entry::Name { name }) if name == "test"));
    }
    #[test]
    fn a_brake_error_followed_by_turn_done_finalizes_once() {
        // The runner sends Error and then TurnDone for the same round when the
        // tool-round ceiling fires. The second finalize used to find the
        // buffers drained and append — and persist — a phantom "(无输出)"
        // reply, so the user saw a bogus bubble after every brake.
        let mut s = st();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::Error("工具调用轮数撞上限".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let replies: Vec<String> = s
            .transcript()
            .iter()
            .filter_map(|e| match e {
                Entry::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(replies, vec!["答案".to_string()], "一轮只应有一条回复");
    }

    #[test]
    fn an_empty_reply_persists_the_shared_placeholder() {
        // The other half of the byte-fidelity contract: whatever the loop puts
        // in the live context for an empty reply, the entry says the same.
        let mut s = st();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::server::ai::types::StopReason::Stop,
        ));
        let reply = s
            .transcript()
            .iter()
            .find_map(|e| match e {
                Entry::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(reply, crate::server::entry::empty_reply());
    }

    #[test]
    fn in_memory_mode_has_no_persistence() {
        let mut s = st();
        assert!(s.persistence().is_none());
        assert!(s.ensure_session(std::path::Path::new("/tmp")).is_none());
    }
}
