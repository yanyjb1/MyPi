//! Signal protocol — everything that can dirty a region, on one channel.
//!
//! The main loop is **signal-driven, not polled**: the input thread
//! forwards crossterm events and the socket reader thread forwards wire
//! messages, and the loop blocks on `recv()` until something arrives. Zero
//! CPU while idle. A dirty region is marked idempotently — a burst of
//! frames coalesces into one repaint — and ratatui's internal double
//! buffer diff means only changed cells reach the terminal.

/// One unit of "something happened". Regions decide for themselves
/// whether a signal dirties them; the loop never inspects the payload.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // ServerMsg 天然大：Box 化徒增分配，信号即弃，无实害
pub enum Signal {
    // clippy::large_enum_variant: ServerMsg 携带转录条目，天然比按键事件大
    // 一个量级。Box 化会让每条消息多一次分配；信号通道在本循环内、批量
    // 处理后即弃，大小差异无实际代价，故保留未装箱。
    /// A semantic action translated from keyboard/paste input (input
    /// thread → main loop; translation needs the current KeyContext,
    /// which only the main thread can build).
    ///
    /// Why translate in the main thread: `translate_with` must see
    /// `KeyContext` (popup open? streaming?) — stale context in the
    /// sender would misroute keys. The thread forwards raw events; the
    /// loop translates at the moment of consumption.
    Key(ratatui::crossterm::event::KeyEvent),
    /// Paste arrives as its own event (bracketed paste).
    Paste(String),
    /// Mouse event; routed by hit-testing the last frame's layout.
    Mouse(ratatui::crossterm::event::MouseEvent),
    /// A fresh workspace git snapshot from the poller thread. Borrowed: the
    /// poller owns the value; components clone what they keep (the git
    /// segment already does). `None` = outside a repository (segment hides).
    Git(Option<crate::git::GitStatus>),
    /// Terminal resized: all width-dependent caches are invalid.
    Resized,
    /// A message pushed by the daemon (socket reader thread → main loop).
    /// Dispatched to the `SessionView`, which maps it onto zone calls.
    Server(crate::server::wire::ServerMsg),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_enum_covers_all_sources() {
        // 信号源完整性：键/粘贴/鼠标/resize/服务端推送 五路都必须能构造。
        use ratatui::crossterm::event::{KeyCode, KeyEvent as K, KeyModifiers};
        let k = K::new(KeyCode::Char('a'), KeyModifiers::NONE);
        let _ = Signal::Key(k);
        let _ = Signal::Paste("x".into());
        let _ = Signal::Resized;
        let _ = Signal::Server(crate::server::wire::ServerMsg::HelloOk {
            proto: 1,
            commands: Vec::new(),
        });
    }
}
