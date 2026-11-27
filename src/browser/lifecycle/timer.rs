//! One timer for the process: close idle tabs, then close an idle browser.

use std::sync::LazyLock;
use std::time::Duration;

use super::super::tabs;

static START: LazyLock<()> = LazyLock::new(|| {
    std::thread::Builder::new()
        .name("web-timer".to_string())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                super::idle::sweep();
                tabs::reap();
            }
        })
        .expect("cannot start the web timer thread");
});

pub(crate) fn start() {
    LazyLock::force(&START);
}
