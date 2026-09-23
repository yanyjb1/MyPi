//! Completion controller — the input zone's completion surface, extracted
//! from `App` (which keeps only a `completion` field).
//!
//! Owns the popup state machine plus the "which word did we compute
//! candidates for" memo. Leaf-state rules live in `crate::tui::leaf`
//! (pure, unit-tested); this module only wires them to the editor text
//! and the runtime config (model ids come from `Config`).

use crate::ai::config::Config;
use super::leaf;
use super::engine::{self, CompletionPopup};

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
}

pub struct CompletionController {
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
    pub fn new() -> Self {
        Self { popup: CompletionPopup::default(), last_word: None }
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
        self.popup.locked_height().or(Some(self.popup.items().len()))
    }

    /// The word currently under the cursor, if it is a completion target.
    pub fn current_word(text: &str, cursor: usize) -> Option<String> {
        engine::candidate(text, cursor).map(|(_, word)| word)
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
                    && let Some(spec) = engine::lookup(cmd)
                {
                    match spec.args {
                        engine::ArgKind::ModelId => {
                            if let Some(items) = Self::model_items(cmd, prefix.as_str(), cx) {
                                // Leaf state (single candidate == whole
                                // line): close so Tab/Enter stop
                                // re-confirming the same completion and
                                // Enter submits again.
                                if items.is_empty()
                                    || leaf::leaf_state(&items, prefix_text, true, text).is_some()
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
                        engine::ArgKind::None => {
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
                let mut items = engine::complete(&word, at_start, cx.cwd, cx.home);
                // Complete command followed by a space ("/model ") -> candidates become that command's argument list
                if at_start
                    && items.is_empty()
                    && let Some(arg) = Self::model_items_from_word(&word, cx)
                {
                    items = arg;
                }
                // Final-stage detection: the single leaf rule lives in
                // `leaf::leaf_state` (file leaf / exact command / argument
                // line-leaf). Close there -> Enter submits directly.
                if items.is_empty() || leaf::leaf_state(&items, &word, at_start, text).is_some() {
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
            .filter(|m| m.id.starts_with(arg_part))
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

    fn model_items_from_word(word: &str, cx: &InputCtx) -> Option<Vec<engine::Completion>> {
        let (cmd, arg_part) = word.split_once(' ')?;
        let spec = engine::lookup(cmd)?;
        if matches!(spec.args, engine::ArgKind::ModelId) {
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

/// Extract the model candidate list from a config handle (id + display
/// name; the display name is what the popup shows in gray).
pub fn models_from_config(
    cfg: &Option<std::rc::Rc<std::cell::RefCell<Config>>>,
) -> Option<Vec<ModelCandidate>> {
    let cfg = cfg.as_ref()?;
    let cfg = cfg.borrow();
    Some(
        cfg.models()
            .map(|(p, m)| ModelCandidate {
                provider: p.to_string(),
                id: m.id.clone(),
                detail: Config::display_name(m).to_string(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

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
        c.refresh(&cx("/model gl", 9));
        // "/model gl" enters argument mode; the id list is filtered by
        // "gl" and the insert text carries the full provider:id address.
        assert!(c.is_open());
        let items = c.popup.items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "/model local:global:m1");
    }
}
