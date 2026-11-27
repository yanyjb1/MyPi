//! Exit signals: close the browser before the process goes away.

use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::sync::LazyLock;

static START: LazyLock<()> = LazyLock::new(|| {
    std::thread::Builder::new()
        .name("web-signal".to_string())
        .spawn(|| {
            let Ok(mut signals) = Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
                return;
            };
            if let Some(signal) = signals.forever().next() {
                super::shutdown();
                if signal_hook::low_level::emulate_default_handler(signal).is_err() {
                    std::process::exit(128 + signal);
                }
            }
        })
        .expect("cannot start the web signal thread");
});

pub(crate) fn install() {
    LazyLock::force(&START);
}
