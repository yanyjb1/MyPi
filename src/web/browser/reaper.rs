use std::sync::OnceLock;

use super::session;

static START: OnceLock<()> = OnceLock::new();

pub(crate) fn start() {
    START.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                session::reap();
            }
        });
    });
}
