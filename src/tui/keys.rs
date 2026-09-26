//! 键鼠捕捉 —— crossterm 事件 → 归一化事件。
//!
//! 上层（APP）只做捕捉和下发，**不解释语义**。Ctrl+V 是什么意思、
//! Esc 关不关弹窗、↑ 在历史区还是输入区——全部是下层（Zone/子区）
//! 自己的事。这里只回答"物理上发生了什么"。
//!
//! 覆盖范围：键盘、bracketed paste、鼠标滚轮。鼠标其余输入（移动、
//! 左右键）不归一化、不下发（用户决策：现阶段只管滚轮）。

use ratatui::crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};

/// 归一化后的物理事件：只描述"按了什么/粘了什么/滚了哪边"，零语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawEvent {
    /// 一次按键（含修饰键原样携带）。
    Key { key: KeyEvent },
    /// 一次终端粘贴（bracketed paste，可能多行）。
    Paste(String),
    /// 滚轮向上。
    ScrollUp,
    /// 滚轮向下。
    ScrollDown,
}

/// 从 crossterm KeyEvent 归一化。修饰键原样保留，不做任何语义判断。
pub fn normalize(key: KeyEvent) -> RawEvent {
    RawEvent::Key { key }
}

/// 从 crossterm MouseEvent 归一化。只认滚轮；其余鼠标输入返回 None
/// （直接丢弃，不产生事件）。
pub fn normalize_mouse(m: &MouseEvent) -> Option<RawEvent> {
    match m.kind {
        MouseEventKind::ScrollUp => Some(RawEvent::ScrollUp),
        MouseEventKind::ScrollDown => Some(RawEvent::ScrollDown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};

    fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn normalize_carries_key_and_modifiers_verbatim() {
        let ev = normalize(k(KeyCode::Char('v'), KeyModifiers::CONTROL));
        match ev {
            RawEvent::Key { key } => {
                assert_eq!(key.code, KeyCode::Char('v'));
                assert_eq!(key.modifiers, KeyModifiers::CONTROL);
            }
            _ => panic!("必须是 Key"),
        }
    }

    #[test]
    fn normalize_mouse_passes_scroll_drops_rest() {
        let mut m = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(normalize_mouse(&m), Some(RawEvent::ScrollUp));
        m.kind = MouseEventKind::ScrollDown;
        assert_eq!(normalize_mouse(&m), Some(RawEvent::ScrollDown));
        m.kind = MouseEventKind::Moved;
        assert_eq!(normalize_mouse(&m), None, "移动不属于归一化范围");
        m.kind = MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left);
        assert_eq!(normalize_mouse(&m), None, "左右键现阶段不归一化");
    }
}
