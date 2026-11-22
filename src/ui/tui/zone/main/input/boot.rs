// The zone's own life: the startup questions and the draft reset.

use super::zone::InputZone;

// The git segment's scan interval, in seconds.
pub(super) const GIT_SCAN_KEY: &str = "config.ui.tui.zone.main.input.deco.gitScanInterval";

// Whether a session's first message carries the workspace preparation.
pub(super) const AUTO_PREPARE_KEY: &str = "config.ui.tui.zone.main.input.autoPrepare";

// The currency the cost pill counts in.
pub(super) const FX_CURRENCY_KEY: &str = "config.ui.tui.fx.currency";

// The model a draft opens with; a session row overrides it once there is one.
pub(super) const DEFAULT_MODEL_KEY: &str = "config.server.api.defaultModel";

// What `~` stands for; the server answers it, resolving an unset key to the platform's own home.
pub(super) const HOME_KEY: &str = "config.server.platform.home";

// The display language the panel and number formats follow.
pub(super) const LANG_KEY: &str = crate::ui::tui::lang::Lang::KEY;

impl InputZone {
    // First frame: the process-wide facts a draft can ask for on its own.
    pub fn boot(&mut self) {
        if self.booted {
            return;
        }
        self.booted = true;
        for key in [
            GIT_SCAN_KEY,
            AUTO_PREPARE_KEY,
            FX_CURRENCY_KEY,
            DEFAULT_MODEL_KEY,
            LANG_KEY,
            HOME_KEY,
        ] {
            self.want(key);
        }
    }

    // Drop the draft and every transient interaction. Submission history stays.
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
    }
}
