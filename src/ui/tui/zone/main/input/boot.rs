//! The zone's own life: the startup questions, the draft reset, and what outlives a session.

use super::Data;
use super::zone::InputZone;
use crate::ui::tui::zone::Request;

/// The git segment's scan interval, in seconds.
pub(super) const GIT_SCAN_KEY: &str = "config.ui.tui.zone.main.input.deco.gitScanInterval";

/// The currency the cost pill counts in.
pub(super) const FX_CURRENCY_KEY: &str = "config.ui.tui.fx.currency";

/// The model a draft opens with; a session row overrides it once there is one.
pub(super) const DEFAULT_MODEL_KEY: &str = "config.server.api.defaultModel";

/// The display language the panel and number formats follow.
pub(super) const LANG_KEY: &str = "config.ui.lang";

impl InputZone {
    /// First frame: the process-wide facts a draft can ask for on its own.
    pub fn boot(&mut self) {
        if self.booted {
            return;
        }
        self.booted = true;
        for key in [GIT_SCAN_KEY, FX_CURRENCY_KEY, DEFAULT_MODEL_KEY, LANG_KEY] {
            self.outbox.push(Request::Config {
                key: key.to_string(),
            });
        }
    }

    /// Drop one session's readings. What the process owns stays: the workspace, the git
    /// scan interval, the display currency and the language outlive any one conversation.
    pub(super) fn reset_data(&mut self) {
        let cwd = std::mem::take(&mut self.data.cwd);
        let git_scan = self.data.git_scan;
        let currency = self.data.money.currency;
        let lang = self.data.lang;
        self.data = Data::default();
        self.data.cwd = cwd;
        self.data.git_scan = git_scan;
        self.data.money.currency = currency;
        self.data.lang = lang;
    }

    /// Drop the draft and every transient interaction. Submission history stays.
    pub fn clear(&mut self) {
        self.editor.clear();
        self.history.exit();
        self.finished = true;
        self.scroll = 0;
        self.pending_hire = None;
        self.hang = false;
        self.esc_armed = None;
        self.enter_armed = None;
        self.pending_exits.clear();
        self.queue.clear();
        self.in_flight = None;
        self.pending_notice.clear();
        self.pending_name = None;
    }
}
