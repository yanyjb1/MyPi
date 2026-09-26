//! `/resume` —— 全屏会话选择器。
//!
//! 布局照 omp 的 `overlays/session-selector.ts`：标题栏 + 搜索框 +
//! 一条会话一块（标题行 / 首条消息预览 / 元信息行）+ 底部按键提示；
//! 框线用我们自己的卡片原语（`render::cards`），全应用一套视觉。
//!
//! 零会话状态：列表从服务端来（[`crate::server::wire::ServerMsg::Sessions`]），
//! 附着请求发回去（[`crate::server::wire::ClientMsg::Attach`]）。Zone 只认识
//! [`Request`]——发不发、怎么发是主循环的事。
//!
//! 作用域：默认只看**当前目录**跑过的会话（项目 A 不该看见项目 B 的），
//! Tab 切到全部。这是 omp 的两个作用域（"current folder" / all）。

use crate::server::wire::SessionInfo;
use crate::tui::keys::RawEvent;
use crate::tui::zone::{Handover, TermSize, ZoneId};

mod render;

/// 列表作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 只在当前目录跑过的会话。
    Current,
    /// 本机全部会话。
    All,
}

impl Scope {
    fn toggled(self) -> Self {
        match self {
            Scope::Current => Scope::All,
            Scope::All => Scope::Current,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Current => "当前目录",
            Scope::All => "全部目录",
        }
    }
}

/// 页面要主循环替它做的事。
///
/// Zone 拿不到 socket（Zone 是纯 UI），所以"要列表""要附着"先记在这里，
/// 主循环 deliver 之后取走执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// 拉一次列表；`under` = 项目作用域的目录（None = 全部）。
    List { under: Option<String> },
    /// 用户选中了某条：附着过去。
    Attach(i64),
    /// 用户确认删除某条（两段式：Delete 键只是亮出来，Enter 才真删）。
    Delete(i64),
}

/// 一行要画的东西（会话 + 已经算好的年龄）。
struct Row {
    info: SessionInfo,
    /// 秒；`None` = 戳坏了，画原文。
    age: Option<u64>,
}

pub struct ResumeZone {
    size: Option<TermSize>,
    rows: Vec<Row>,
    /// `rows` 里通过筛选的下标（筛选不搬数据，只留下标）。
    visible: Vec<usize>,
    filter: String,
    /// `visible` 里的位置。
    selected: usize,
    /// 物理行窗口起点（渲染层按高度算完回写）。
    scroll: usize,
    scope: Scope,
    cwd: std::path::PathBuf,
    /// 收到过列表没有：区分"空"和"还没来"。
    loaded: bool,
    /// Delete 按下后待确认的那条（Enter 才真删；**任何别的操作**都撤销）。
    pending_delete: Option<i64>,
    /// 服务端报错（比如删除被拒）：显示在列表上方，下次列表刷新时清掉。
    error: Option<String>,
    /// 排队等主循环执行的事（删除要连带重拉列表，所以是队列不是单槽）。
    pending: Vec<Request>,
}

impl Default for ResumeZone {
    fn default() -> Self {
        Self {
            size: None,
            rows: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            selected: 0,
            scroll: 0,
            scope: Scope::Current,
            cwd: std::path::PathBuf::from("."),
            loaded: false,
            pending_delete: None,
            error: None,
            pending: Vec::new(),
        }
    }
}

impl ResumeZone {
    /// 进页面：记下当前目录（作用域用），并索要列表。
    ///
    /// 每次进来都重拉：列表是"外面的事实"，上次的内存快照可能已经过期
    /// （别的终端刚存了会话）。
    pub fn begin(&mut self, cwd: std::path::PathBuf) {
        self.cwd = cwd;
        self.loaded = false;
        self.scroll = 0;
        self.pending_delete = None;
        self.error = None;
        // 每次进来都是新的搜索：留着上次的关键字，用户会以为"会话没了"。
        self.filter.clear();
        self.pending.push(Request::List {
            under: self.under(),
        });
    }

    /// 服务端给的列表到了。
    pub fn on_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.loaded = true;
        self.pending_delete = None;
        self.error = None;
        self.rows = sessions
            .into_iter()
            .map(|info| Row {
                age: crate::server::store::age_seconds(&info.started_at),
                info,
            })
            .collect();
        self.refilter();
    }

    /// 服务端报错（删除被拒之类）。选择器持有屏幕时错误得显示在这里，
    /// 不然用户按了 Delete + Enter 之后什么都看不到。
    pub fn on_error(&mut self, message: String) {
        self.pending_delete = None;
        self.error = Some(message);
    }

    /// 取走要主循环执行的事（可能不止一条：删除连带重拉列表）。
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.pending)
    }

    fn under(&self) -> Option<String> {
        match self.scope {
            Scope::Current => Some(self.cwd.to_string_lossy().to_string()),
            Scope::All => None,
        }
    }

    fn row(&self) -> Option<&Row> {
        self.visible.get(self.selected).and_then(|i| self.rows.get(*i))
    }

    /// 选中的会话 id（Enter 用）。**按筛选后的下标取**——在筛选结果里
    /// 用原列表下标取错行是这个页面最容易犯的错。
    pub fn selected_id(&self) -> Option<i64> {
        self.row().map(|r| r.info.id)
    }

    /// 重算可见集合。筛选词按大小写不敏感的子串匹配标题、名字、首条
    /// 消息和目录——够用，且能解释（模糊匹配打不中时用户无从理解）。
    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| needle.is_empty() || r.haystack().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        // 选中位置夹回范围内（筛选后原来的位置可能已经不存在）。
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        self.scroll = 0;
    }

    fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }
}

impl Row {
    /// 筛选用的干草堆：一条会话所有能被认出来的文字。
    fn haystack(&self) -> String {
        let i = &self.info;
        format!(
            "{} {} {} {}",
            i.id,
            i.name.as_deref().unwrap_or(""),
            i.first_message.as_deref().unwrap_or(""),
            i.cwd.as_deref().unwrap_or("")
        )
        .to_lowercase()
    }

    /// 第一行显示什么（照 omp）：显式名字 → 名字；没名字 → **首条消息**
    /// ——用户挑会话问的是"这是哪次对话"，答案就是它。
    ///
    /// 两样都没有（空的草稿会话）才退回合成名 `MM-DD:HH-MMSS+前 7 个字`，
    /// 至少给个能认出来的标签。
    fn title(&self) -> String {
        if let Some(name) = &self.info.name {
            return name.clone();
        }
        if let Some(first) = self.preview_text() {
            return first;
        }
        let meta = crate::server::store::SessionMeta {
            id: self.info.id,
            name: None,
            started_at: self.info.started_at.clone(),
            cwd: self.info.cwd.clone(),
        };
        crate::server::store::display_name(&meta, None)
    }

    /// 预览行：**只有显式命名的会话**才画首条消息（没名字时它已经是标题，
    /// 再画一遍是重复）。
    fn preview(&self) -> Option<String> {
        self.info.name.as_ref()?;
        self.preview_text()
    }

    /// 首条消息的第一行（多行只取第一行，选择器里不画多行预览）。
    fn preview_text(&self) -> Option<String> {
        let text = self.info.first_message.as_deref()?;
        let first = text.lines().next().unwrap_or("").trim();
        (!first.is_empty()).then(|| first.to_string())
    }
}

impl crate::tui::zone::Zone for ResumeZone {
    fn attach(&mut self, size: TermSize) {
        self.size = Some(size);
    }

    fn on_resize(&mut self, change: crate::tui::zone::SizeChange) {
        self.size = Some(change.to);
        self.scroll = 0; // 宽度变了，行高重排
    }

    fn deliver(&mut self, event: RawEvent) -> Option<Handover> {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        match event {
            RawEvent::ScrollUp => self.move_by(-1),
            RawEvent::ScrollDown => self.move_by(1),
            RawEvent::Paste(text) => {
                // 粘贴当筛选词：一个会话选择器不值得为多行粘贴设计语义。
                self.filter.push_str(&text.replace('\n', " "));
                self.refilter();
            }
            RawEvent::Key { key } => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                // 换个地方、翻个页、打个字都算"别的操作"：确认态撤销。
                // 只有 Enter 保留它（下面先看确认态）。
                if !matches!(key.code, KeyCode::Enter | KeyCode::Delete | KeyCode::Esc) {
                    self.pending_delete = None;
                }
                match key.code {
                    KeyCode::Delete => {
                        self.pending_delete = self.selected_id();
                        return None;
                    }
                    KeyCode::Esc => {
                        // Esc 把所有权还给主区：附着与否，进去时是什么样，
                        // 出来时还是什么样（这个页面没有任何副作用）。
                        return Some(Handover {
                            new_owner: ZoneId::Main,
                        });
                    }
                    KeyCode::Enter => {
                        let id = self.selected_id()?;
                        // 两段式：Delete 亮出的那条，Enter 才是"真删"。
                        if self.pending_delete == Some(id) {
                            self.pending_delete = None;
                            self.pending.push(Request::Delete(id));
                            // 删完自己重拉一次：列表是外面的事实，不猜。
                            self.pending.push(Request::List {
                                under: self.under(),
                            });
                            return None;
                        }
                        self.pending.push(Request::Attach(id));
                        return Some(Handover {
                            new_owner: ZoneId::Main,
                        });
                    }
                    KeyCode::Up => self.move_by(-1),
                    KeyCode::Down => self.move_by(1),
                    KeyCode::PageUp => self.move_by(-PAGE),
                    KeyCode::PageDown => self.move_by(PAGE),
                    KeyCode::Home => self.selected = 0,
                    KeyCode::End => self.selected = self.visible.len().saturating_sub(1),
                    KeyCode::Tab => {
                        self.scope = self.scope.toggled();
                        self.pending.push(Request::List {
                            under: self.under(),
                        });
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.refilter();
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.filter.clear();
                        self.refilter();
                    }
                    KeyCode::Char(c) if !ctrl => {
                        self.filter.push(c);
                        self.refilter();
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        render::lines(self)
    }
}

impl ResumeZone {
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }

    pub(crate) fn scope_label(&self) -> &'static str {
        self.scope.label()
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    /// 待确认删除的那条（渲染层要在标题后面画提示）。
    pub(crate) fn pending_delete(&self) -> Option<i64> {
        self.pending_delete
    }

    /// 服务端报错（红色一行）。
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn set_scroll(&mut self, top: usize) {
        self.scroll = top;
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    pub(crate) fn row_at(&self, i: usize) -> Option<RowViewRef<'_>> {
        let row = self.visible.get(i).and_then(|k| self.rows.get(*k))?;
        Some(RowViewRef { row })
    }
}

/// 渲染层要的那几个字段（行本身是私有的，不给外面看全）。
pub(crate) struct RowViewRef<'a> {
    row: &'a Row,
}

impl RowViewRef<'_> {
    pub(crate) fn id(&self) -> i64 {
        self.row.info.id
    }

    pub(crate) fn title(&self) -> String {
        self.row.title()
    }
    pub(crate) fn preview(&self) -> Option<String> {
        self.row.preview()
    }
    pub(crate) fn age(&self) -> Option<u64> {
        self.row.age
    }
    pub(crate) fn stamp(&self) -> &str {
        &self.row.info.started_at
    }
    pub(crate) fn bytes(&self) -> i64 {
        self.row.info.bytes
    }
    pub(crate) fn cwd(&self) -> Option<&str> {
        self.row.info.cwd.as_deref()
    }
}

/// 一页翻几条。
const PAGE: i64 = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::zone::Zone as _;

    fn info(id: i64, name: Option<&str>, first: Option<&str>, cwd: &str) -> SessionInfo {
        SessionInfo {
            id,
            name: name.map(str::to_string),
            started_at: "2026-09-22 14:30:05".into(),
            cwd: Some(cwd.into()),
            first_message: first.map(str::to_string),
            bytes: 1024,
        }
    }

    fn zone(sessions: Vec<SessionInfo>) -> ResumeZone {
        let mut z = ResumeZone::default();
        z.attach(TermSize {
            cols: 80,
            rows: 24,
        });
        z.begin(std::path::PathBuf::from("/proj/a"));
        z.on_sessions(sessions);
        z
    }

    fn key(code: ratatui::crossterm::event::KeyCode) -> RawEvent {
        RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent::new(
                code,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
        }
    }

    fn ctrl(c: char) -> RawEvent {
        RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Char(c),
                ratatui::crossterm::event::KeyModifiers::CONTROL,
            ),
        }
    }

    #[test]
    fn entering_asks_for_the_current_folder_scope() {
        let mut z = ResumeZone::default();
        z.begin(std::path::PathBuf::from("/proj/a"));
        assert_eq!(
            z.take_requests(),
            vec![Request::List {
                under: Some("/proj/a".into())
            }],
            "默认只看当前目录——项目 A 不该看见项目 B 的会话"
        );
    }

    #[test]
    fn enter_attaches_the_row_the_filter_left_under_the_cursor() {
        // 这是本页最容易犯的错：筛选后拿原列表的下标去取行。
        let mut z = zone(vec![
            info(3, Some("丙"), None, "/proj/a"),
            info(2, Some("乙"), None, "/proj/a"),
            info(1, Some("甲"), None, "/proj/a"),
        ]);
        z.take_requests(); // 进页面那次 List
        for c in "乙".chars() {
            z.deliver(key(ratatui::crossterm::event::KeyCode::Char(c)));
        }
        assert_eq!(z.visible_len(), 1, "只剩一条");
        z.deliver(key(ratatui::crossterm::event::KeyCode::Enter));
        assert_eq!(
            z.take_requests(),
            vec![Request::Attach(2)],
            "附着的必须是筛选后光标下的那条（id=2），不是列表第 0 条的 id=3"
        );
    }

    /// 两段式删除：Delete 只是亮出确认，Enter 才真删；**别的操作**撤销。
    ///
    /// 一次按键就删掉一段对话是不可接受的（没有回收站），所以确认态必须
    /// 能被任何别的操作打断——这条就是那个契约。
    #[test]
    fn deleting_takes_two_keys_and_any_other_key_cancels_it() {
        let mut z = zone(vec![
            info(2, Some("乙"), None, "/proj/a"),
            info(1, Some("甲"), None, "/proj/a"),
        ]);
        z.take_requests(); // 进页面那次 List

        // 第一下：只亮确认，什么都不发。
        z.deliver(key(ratatui::crossterm::event::KeyCode::Delete));
        assert_eq!(z.pending_delete(), Some(2), "光标下那条待确认");
        assert!(z.take_requests().is_empty(), "确认态不许发删除请求");

        // 别的操作撤销它。
        z.deliver(key(ratatui::crossterm::event::KeyCode::Down));
        assert_eq!(z.pending_delete(), None, "翻页就算撤销");
        z.deliver(key(ratatui::crossterm::event::KeyCode::Delete));
        z.deliver(key(ratatui::crossterm::event::KeyCode::Up));
        assert_eq!(z.pending_delete(), None, "换一条也算撤销");

        // 确认：发删除 + 重拉列表，且**不附着**（所有权不动）。
        z.deliver(key(ratatui::crossterm::event::KeyCode::Home));
        z.deliver(key(ratatui::crossterm::event::KeyCode::Delete));
        let handover = z.deliver(key(ratatui::crossterm::event::KeyCode::Enter));
        assert_eq!(handover, None, "删除不切屏");
        assert_eq!(
            z.take_requests(),
            vec![
                Request::Delete(2),
                Request::List {
                    under: Some("/proj/a".into())
                }
            ],
            "删完自己重拉：列表是外面的事实"
        );
        assert_eq!(z.pending_delete(), None);
    }

    #[test]
    fn a_server_refusal_is_shown_and_clears_the_confirm() {
        let mut z = zone(vec![info(1, Some("甲"), None, "/proj/a")]);
        z.deliver(key(ratatui::crossterm::event::KeyCode::Delete));
        z.on_error("会话 1 正开着（先离开它再删）".into());
        assert_eq!(z.pending_delete(), None);
        assert!(z.error().unwrap().contains("正开着"));
        z.on_sessions(vec![info(1, Some("甲"), None, "/proj/a")]);
        assert_eq!(z.error(), None, "列表刷新后错误行收掉");
    }

    #[test]
    fn esc_hands_ownership_back_without_attaching() {
        let mut z = zone(vec![info(1, Some("甲"), None, "/proj/a")]);
        z.take_requests(); // 进页面时那次 List，先取走
        let h = z.deliver(key(ratatui::crossterm::event::KeyCode::Esc));
        assert_eq!(h.map(|h| h.new_owner), Some(ZoneId::Main));
        assert!(z.take_requests().is_empty(), "Esc 不附着");
    }

    #[test]
    fn tab_switches_scope_and_keeps_the_filter() {
        let mut z = zone(vec![info(1, Some("甲"), None, "/proj/a")]);
        z.take_requests(); // 进页面那次 List
        z.deliver(key(ratatui::crossterm::event::KeyCode::Char('甲')));
        z.deliver(key(ratatui::crossterm::event::KeyCode::Tab));
        assert_eq!(z.scope(), Scope::All);
        assert_eq!(z.filter(), "甲", "切作用域不该把用户打的字吃掉");
        assert_eq!(z.take_requests(), vec![Request::List { under: None }]);
        z.deliver(key(ratatui::crossterm::event::KeyCode::Tab));
        assert_eq!(
            z.take_requests(),
            vec![Request::List {
                under: Some("/proj/a".into())
            }]
        );
    }

    #[test]
    fn navigation_clamps_and_never_wraps() {
        let mut z = zone(vec![
            info(1, Some("甲"), None, "/proj/a"),
            info(2, Some("乙"), None, "/proj/a"),
        ]);
        z.deliver(key(ratatui::crossterm::event::KeyCode::Up));
        assert_eq!(z.selected_id(), Some(1), "首个再往上还是首个");
        for _ in 0..5 {
            z.deliver(key(ratatui::crossterm::event::KeyCode::Down));
        }
        assert_eq!(z.selected_id(), Some(2), "末尾再往下还是末尾");
        z.deliver(key(ratatui::crossterm::event::KeyCode::Home));
        assert_eq!(z.selected_id(), Some(1));
        z.deliver(key(ratatui::crossterm::event::KeyCode::End));
        assert_eq!(z.selected_id(), Some(2));
    }

    #[test]
    fn the_filter_survives_a_filter_that_matches_nothing_then_backspace_restores_it() {
        let mut z = zone(vec![
            info(1, Some("甲"), None, "/proj/a"),
            info(2, Some("乙"), None, "/proj/a"),
        ]);
        z.deliver(key(ratatui::crossterm::event::KeyCode::Char('没')));
        assert_eq!(z.visible_len(), 0);
        z.deliver(key(ratatui::crossterm::event::KeyCode::Backspace));
        assert_eq!(z.visible_len(), 2, "退格把筛选词去掉，列表回来");
        // 空筛选再退格：不许 panic（多字节/空串边界）。
        z.deliver(key(ratatui::crossterm::event::KeyCode::Backspace));
        assert_eq!(z.filter(), "");
        z.deliver(ctrl('u'));
        assert_eq!(z.filter(), "");
    }

    #[test]
    fn unnamed_sessions_show_their_first_message_as_the_title() {
        let z = zone(vec![info(7, None, Some("帮我把 todo 改成有状态的"), "/proj/a")]);
        let row = z.row_at(0).unwrap();
        assert_eq!(
            row.title(),
            "帮我把 todo 改成有状态的",
            "没名字时标题就是首条消息（omp 的规则）"
        );
        assert_eq!(row.preview(), None, "标题已经是首条消息，不再重复画");
        // 空的草稿会话：名字和消息都没有，退回合成名，至少能认。
        let empty = zone(vec![info(9, None, None, "/proj/a")]);
        assert!(
            empty.row_at(0).unwrap().title().starts_with("09-22:14-3005"),
            "两样都没有才用合成名：{}",
            empty.row_at(0).unwrap().title()
        );
        let named = zone(vec![info(8, Some("给会话起的名字"), Some("首条消息"), "/proj/a")]);
        let row = named.row_at(0).unwrap();
        assert_eq!(row.title(), "给会话起的名字");
        assert_eq!(row.preview().as_deref(), Some("首条消息"), "有名字时预览行才画");
    }
}
