//! Completion controller — the input zone's completion surface, extracted
//! from `App` (which keeps only a `completion` field).
//!
//! Owns the popup state machine plus the "which word did we compute
//! candidates for" memo. Leaf-state rules live in `crate::tui::leaf`
//! (pure, unit-tested); this module only wires them to the editor text
//! and the runtime config (model ids come from `Config`).

use super::engine::{self, CompletionPopup};
use super::leaf;

/// One model candidate as the controller sees it (id for the wire,
/// detail for the popup's gray text).
#[derive(Debug, Clone)]
pub struct ModelCandidate {
    pub provider: String,
    pub id: String,
    pub detail: String,
}

/// Input the controller needs to recompute candidates: the whole editor
/// text, the cursor, and where path expansion resolves from.
pub struct InputCtx<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub cwd: &'a std::path::Path,
    pub home: &'a std::path::Path,
    /// Model-id candidates (`/model`, `/switch`): resolved against the
    /// live config. None = config unavailable (no model completion).
    pub models: Option<Vec<ModelCandidate>>,
    /// Profile 花名册（`/profile` 的候选）。来源同 models：本地配置。
    pub profiles: &'a [String],
}

pub struct CompletionController {
    /// 命令表：由服务端 `hello_ok` 下发（见 `engine::complete_commands`）。
    /// 空表 = 还没收到，补全只给路径。
    commands: Vec<crate::server::wire::CommandInfo>,
    popup: CompletionPopup,
    // The last "path word" candidates were computed for (start + text).
    //
    // Without the memo, every cursor move re-enters `refresh` and
    // re-scans the directory via `read_dir` whenever the input contains
    // a path-like word — even though moving the cursor usually does not
    // change the word. Skip the rescan when the word is unchanged.
    last_word: Option<(usize, String)>,
}

impl CompletionController {
    /// 服务端把命令表送来了（握手时一次）。
    pub fn set_commands(&mut self, commands: Vec<crate::server::wire::CommandInfo>) {
        self.commands = commands;
        self.popup.close();
    }

    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            popup: CompletionPopup::default(),
            last_word: None,
        }
    }

    // ---- read side (renderer / key routing) ----

    pub fn popup(&self) -> &CompletionPopup {
        &self.popup
    }

    pub fn is_open(&self) -> bool {
        self.popup.is_open()
    }

    /// Reserved-area height the popup claims (row lock while open).
    pub fn reserved_height(&self) -> Option<usize> {
        self.popup
            .locked_height()
            .or(Some(self.popup.items().len()))
    }

    // ---- write side (state machine) ----

    /// Recompute candidates after an input change. A wordless cursor
    /// closes the popup — which is why it vanishes on cursor moves.
    pub fn refresh(&mut self, cx: &InputCtx) {
        let text = cx.text;
        let cursor = cx.cursor;

        // Command **argument** mode: line starts with `/` and the cursor
        // is past the command name and a space ("/model ", "/model gl").
        // candidate() stops at whitespace, so this branch must handle it
        // or the popup would never open after a complete command.
        if text.starts_with('/') {
            // `cursor` is a byte offset; multibyte characters (CJK input)
            // require slicing at a char boundary — slicing through a
            // character panics (it has happened).
            let mut end = cursor.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            let prefix_text = &text[..end];
            if !prefix_text.contains('\n') {
                let prefix: String = prefix_text.to_string();
                if prefix.contains(' ')
                    && let Some((cmd, _)) = prefix.split_once(' ')
                    && let Some(kind) =
                        engine::find(&self.commands, cmd).map(engine::ArgKind::of)
                {
                    match kind {
                        engine::ArgKind::ModelId => {
                            if let Some(items) = Self::model_items(cmd, prefix.as_str(), cx) {
                                // Leaf state (single candidate == whole
                                // line): close so Tab/Enter stop
                                // re-confirming the same completion and
                                // Enter submits again.
                                if items.is_empty()
                                    || leaf::leaf_state(&items, prefix_text, true, text, &self.commands).is_some()
                                {
                                    self.close();
                                } else {
                                    self.last_word = None;
                                    self.popup.open(items, 0, cursor);
                                }
                                return;
                            }
                        }
                        // Path argument (/cdp /tm...): falls through to the generic file path completion
                        engine::ArgKind::Path => {}
                        // No-argument commands (/name x...): the argument is not a completion target; close the popup
                        // 无参数命令（`/q`）：命令名就是终点。
                        engine::ArgKind::None => {
                            self.close();
                            return;
                        }
                        engine::ArgKind::ProfileName => {
                            let items = Self::profile_items(cmd, prefix.as_str(), cx);
                            if items.is_empty()
                                || leaf::leaf_state(&items, prefix_text, true, text, &self.commands)
                                    .is_some()
                            {
                                self.close();
                            } else {
                                self.last_word = None;
                                self.popup.open(items, 0, cursor);
                            }
                            return;
                        }
                        // 自由文本参数（`/name 我的会话`）：没有候选可给。
                        engine::ArgKind::Text => {
                            self.close();
                            return;
                        }
                    }
                }
                // Incomplete command names ("/mod") and path-argument commands: handled by the generic logic below.
            }
        }

        match engine::candidate(text, cursor) {
            Some((from, word)) => {
                // Same word (pure cursor movement): skip the disk rescan.
                // The comparison includes `from`: the same word at a
                // different position needs a different replacement index.
                if self.last_word.as_ref() == Some(&(from, word.clone())) && self.popup.is_open() {
                    return;
                }
                self.last_word = Some((from, word.clone()));
                let at_start = from == 0;
                let mut items = engine::complete(&word, at_start, cx.cwd, cx.home, &self.commands);
                // Complete command followed by a space ("/model ") -> candidates become that command's argument list
                if at_start
                    && items.is_empty()
                    && let Some(arg) = Self::model_items_from_word(&word, cx, &self.commands)
                {
                    items = arg;
                }
                // Final-stage detection: the single leaf rule lives in
                // `leaf::leaf_state` (file leaf / exact command / argument
                // line-leaf). Close there -> Enter submits directly.
                if items.is_empty() || leaf::leaf_state(&items, &word, at_start, text, &self.commands).is_some() {
                    self.close();
                } else {
                    self.popup.open(items, from, cursor);
                }
            }
            None => {
                self.close();
            }
        }
    }

    /// Tab: advance the completion (bash/editor semantics).
    ///
    /// 1. popup closed -> open. A single candidate applies at once;
    ///    multiple candidates are only listed, text untouched.
    /// 2. popup open -> extend the common prefix; if that cannot
    ///    advance, confirm the highlighted entry.
    ///
    /// Returns the completion action to apply (or None: nothing changed).
    pub fn on_tab(&mut self, cx: &InputCtx, editor_text: &str) -> Option<engine::CompletionAction> {
        if !self.popup.is_open() {
            self.refresh(cx);
            if !self.popup.is_open() {
                return None; // no candidates at all
            }
            if self.popup.items().len() == 1 {
                return Some(self.popup.accept());
            }
            return None; // listed, not applied — second Tab confirms
        }
        // Open: try the common prefix first.
        if let Some(action) = self.popup.accept_common_prefix(editor_text) {
            return Some(action);
        }
        Some(self.popup.accept())
    }

    /// Explicitly confirm the highlighted entry (Enter while open).
    pub fn accept(&mut self) -> engine::CompletionAction {
        self.popup.accept()
    }

    /// Try extending to the candidates' common prefix first; falls back
    /// to None when the prefix cannot advance.
    pub fn accept_common_prefix(&mut self, current: &str) -> Option<engine::CompletionAction> {
        self.popup.accept_common_prefix(current)
    }

    pub fn move_selection(&mut self, d: isize) {
        self.popup.move_selection(d);
    }

    /// Close and drop the memo (Esc, submit, editor clear...).
    pub fn close(&mut self) {
        self.popup.close();
        self.last_word = None;
    }

    // ---- model-id candidate builders ----

    /// Candidates for the ModelId argument. `word` is the whole
    /// "command + typed argument" string ("/model gl"); `filter` is the
    /// argument part the typed text starts with.
    fn model_items(cmd: &str, word: &str, cx: &InputCtx) -> Option<Vec<engine::Completion>> {
        let models = cx.models.as_ref()?;
        let arg_part = word.split_once(' ')?.1;
        let mut out: Vec<_> = models
            .iter()
            // 两种写法都算命中：光打 id（`model-a`）和完整地址（`fake:model-a`）。
            // 少了后面这一种，Tab 补出公共前缀 `fake:model-` 之后候选会
            // 全部落空、弹窗当场关掉——用户看到的像是"补全把自己弄没了"。
            .filter(|m| {
                arg_part.is_empty()
                    || m.id.starts_with(arg_part)
                    || format!("{}:{}", m.provider, m.id).starts_with(arg_part)
            })
            .map(|m| engine::Completion {
                name: m.id.clone(),
                detail: m.detail.clone(),
                is_dir: false,
                insert: format!("{cmd} {}:{}", m.provider, m.id),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Some(out)
    }

    /// `/profile` 的候选：花名册里的名字（含内置的 `oh-my-pi`）。
    fn profile_items(cmd: &str, word: &str, cx: &InputCtx) -> Vec<engine::Completion> {
        let Some((_, arg_part)) = word.split_once(' ') else {
            return Vec::new();
        };
        cx.profiles
            .iter()
            .filter(|n| n.starts_with(arg_part))
            .map(|n| engine::Completion {
                name: n.clone(),
                detail: String::new(),
                is_dir: false,
                insert: format!("{cmd} {n}"),
            })
            .collect()
    }

    fn model_items_from_word(
        word: &str,
        cx: &InputCtx,
        table: &[crate::server::wire::CommandInfo],
    ) -> Option<Vec<engine::Completion>> {
        let (cmd, arg_part) = word.split_once(' ')?;
        let kind = engine::find(table, cmd).map(engine::ArgKind::of)?;
        if kind == engine::ArgKind::ModelId {
            Self::model_items(cmd, arg_part, cx)
        } else {
            None
        }
    }
}

impl Default for CompletionController {
    fn default() -> Self {
        Self::new()
    }
}

/// 保留区补全服务：CompletionController 的所有者 + 保留区 Claim 的
/// 实现者。作为保留区"可被激活的服务"住在这里——补全弹出时 claim
/// 保留区行高，借走 ↑↓ 移动高亮、Esc 自关。
///
/// Confirm（Tab/Enter 确认候选）**不借**：确认要把选中文本写回编辑器
/// 并重新刷新候选，这条 epilogue 归输入区——输入区对 Complete 动作
/// 直接调用 [`CompletionService::confirm`]，拿回 `CompletionAction`
/// 自己应用到编辑器。
pub struct CompletionService {
    pub controller: CompletionController,
    pub cwd: std::path::PathBuf,
    pub home: std::path::PathBuf,
    pub models: Option<Vec<ModelCandidate>>,
    /// Profile 花名册（`/profile` 的候选）。和 `models` 一样来自本地配置——
    /// 这两样都是**运行时配置事实**，不是会话状态，前端本来就得读。
    pub profiles: Vec<String>,
    /// 光标的终端绝对列（输入区上报）。弹窗渲染锚定/左移用。
    pub anchor_col: usize,
}

impl CompletionService {
    pub fn new(cwd: std::path::PathBuf, home: std::path::PathBuf) -> Self {
        Self {
            controller: CompletionController::new(),
            cwd,
            home,
            models: None,
            profiles: Vec::new(),
            anchor_col: 0,
        }
    }

    /// 从输入区上下文刷新候选。
    /// 服务端把命令表送来了（握手时一次）。
    pub fn set_commands(&mut self, commands: Vec<crate::server::wire::CommandInfo>) {
        self.controller.set_commands(commands);
    }

    /// 模型与 profile 的候选表（启动时一次，来自已加载的配置）。
    ///
    /// 没有它们，`/switch` `/model` `/profile` 的补全就是死的：命令表只
    /// 说"这个参数是什么形状"，合法值得有人送进来。
    pub fn set_candidates(&mut self, models: Vec<ModelCandidate>, profiles: Vec<String>) {
        self.models = Some(models);
        self.profiles = profiles;
    }

    pub fn refresh(&mut self, text: &str, cursor: usize) {
        let cx = InputCtx {
            text,
            cursor,
            cwd: &self.cwd,
            home: &self.home,
            models: self.models.clone(),
            profiles: &self.profiles,
        };
        self.controller.refresh(&cx);
    }

    /// Tab 语义推进（打开/前缀扩展/确认）。返回要应用到编辑器的替换。
    pub fn on_tab(&mut self, text: &str, cursor: usize) -> engine::CompletionAction {
        let cx = InputCtx {
            text,
            cursor,
            cwd: &self.cwd,
            home: &self.home,
            models: self.models.clone(),
            profiles: &self.profiles,
        };
        self.controller
            .on_tab(&cx, text)
            .unwrap_or(engine::CompletionAction::None)
    }
}

impl crate::tui::zone::main::reserved::area::Claim for CompletionService {
    fn id(&self) -> &'static str {
        "completion"
    }

    /// 关着就是 **不占用**（`None`），不是「占 0 行」。
    ///
    /// 返回 `Some(0)` 会把保留区永远攥在手里：strip 在第一次看到这条认领
    /// 时就把行高冻结成 `0.max(1)=1`，之后「进行中的认领保持冻结值」——
    /// 弹窗再开也永远只有一行，候选全被裁掉。
    fn want(&self) -> Option<usize> {
        if !self.controller.is_open() {
            return None;
        }
        self.controller.reserved_height()
    }

    fn max(&self) -> usize {
        crate::tui::zone::main::reserved::area::DEFAULT_MAX
    }

    fn accepts(&self) -> &'static [crate::tui::zone::main::input::semantics::Action] {
        use crate::tui::zone::main::input::semantics::Action;
        // 弹窗只借它交互的键：高亮移动 + 自关。Confirm（Complete）不借
        // ——确认的 epilogue（写回编辑器+再刷新）归输入区。
        &[Action::CompleteUp, Action::CompleteDown, Action::DismissCompletion]
    }

    fn on_key(&mut self, action: &crate::tui::zone::main::input::semantics::Action) -> bool {
        use crate::tui::zone::main::input::semantics::Action;
        match action {
            Action::CompleteUp => {
                self.controller.move_selection(-1);
                true
            }
            Action::CompleteDown => {
                self.controller.move_selection(1);
                true
            }
            Action::DismissCompletion => {
                self.controller.close();
                true
            }
            // 只观察不消费；输入区继续自己的处理。
            _ => false,
        }
    }

    fn draw(
        &self,
        term_w: u16,
        rows: usize,
        p: &crate::tui::theme::Palette,
    ) -> Vec<ratatui::text::Line<'static>> {
        crate::tui::zone::main::reserved::completion::popup::render(
            self.controller.popup(),
            term_w,
            rows,
            self.anchor_col,
            p,
        )
        .lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 花名册：一条命令表的替身 + 两张候选表。
    const PROFILES: [&str; 3] = ["default", "novelist", "oh-my-pi"];

    fn cx<'a>(text: &'a str, cursor: usize) -> InputCtx<'a> {
        InputCtx {
            text,
            cursor,
            cwd: std::path::Path::new("/tmp"),
            home: std::path::Path::new("/tmp"),
            models: Some(vec![ModelCandidate {
                provider: "local".into(),
                id: "global:m1".into(),
                detail: String::new(),
            }]),
            profiles: &PROFILE_NAMES,
        }
    }

    fn profiles() -> Vec<String> {
        PROFILES.iter().map(|s| s.to_string()).collect()
    }

    static PROFILE_NAMES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(profiles);

    #[test]
    fn wordless_cursor_closes_popup() {
        let mut c = CompletionController::new();
        c.refresh(&cx("hello", 5));
        assert!(!c.is_open());
    }

    #[test]
    fn slash_command_without_args_closes() {
        let mut c = CompletionController::new();
        // "/name " -> ArgKind::None -> close
        c.refresh(&cx("/name ", 6));
        assert!(!c.is_open());
    }

    #[test]
    fn model_argument_lists_candidates_with_full_address() {
        let mut c = CompletionController::new();
        // 命令表由服务端下发；测试里喂一份同样的形状。
        c.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/model".into(),
            aliases: vec![],
            detail: "set default model".into(),
            args: "model_id".into(),
            scope: "local".into(),
        }]);
        c.refresh(&cx("/model gl", 9));
        // "/model gl" enters argument mode; the id list is filtered by
        // "gl" and the insert text carries the full provider:id address.
        assert!(c.is_open());
        let items = c.popup.items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "/model local:global:m1");
    }

    /// `/profile` 的候选来自花名册（含内置的 `oh-my-pi`）。
    ///
    /// 命令表只说"这个参数是 profile 名"，合法值得有人送进来——送不进来
    /// 的话 Tab 什么都不会弹（这正是补全"接上"的意思）。
    #[test]
    fn profile_argument_completes_from_the_roster() {
        let mut c = CompletionController::new();
        c.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/profile".into(),
            aliases: vec![],
            detail: "切换 profile".into(),
            args: "profile_name".into(),
            scope: "session".into(),
        }]);
        // 全量：三张名字都该在
        c.refresh(&cx("/profile ", 9));
        assert!(c.is_open(), "有候选就该弹窗");
        let all: Vec<String> = c.popup.items().iter().map(|i| i.name.clone()).collect();
        assert_eq!(all, vec!["default", "novelist", "oh-my-pi"]);

        // 前缀过滤 + insert 是完整一行
        c.refresh(&cx("/profile n", 10));
        let items = c.popup.items().to_vec();
        assert_eq!(items.len(), 1, "只有 novelist 命中");
        assert_eq!(items[0].insert, "/profile novelist");

        // 与模型候选互不串台：`/model ` 拿到的是 id，不是 profile 名。
        c.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/model".into(),
            aliases: vec![],
            detail: "设置默认模型".into(),
            args: "model_id".into(),
            scope: "session".into(),
        }]);
        c.refresh(&cx("/model ", 7));
        let ids: Vec<String> = c.popup.items().iter().map(|i| i.name.clone()).collect();
        assert_eq!(ids, vec!["global:m1"], "模型那条走的是 models");
    }
}
