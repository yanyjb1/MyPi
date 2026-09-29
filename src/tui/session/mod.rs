//! Terminal-session lifecycle — owning the store, the event loop, and
//! the frame pump. app.rs answers "what does this keypress mean"; this
//! domain owns everything from process start to cursor placement.

mod r#loop;
pub mod signal;
pub mod view;

pub use r#loop::run_tui;

/// **临时诊断**：设了 `MYPI_KEYLOG=<路径>` 时把一行追加进那个文件。
///
/// 真机上"按键到判定"这条链只能靠事实说话——读代码只能推出"应该怎样"。
/// 不设变量时只读一次环境变量，之后每次调用是一次 `OnceLock` 读。
pub fn keylog(line: &str) {
    use std::io::Write as _;
    static PATH: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    let path = PATH.get_or_init(|| std::env::var_os("MYPI_KEYLOG").map(std::path::PathBuf::from));
    let Some(path) = path.as_ref() else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
