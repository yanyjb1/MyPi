//! 历史区渲染 —— 画师。
//!
//! 入口一个：[`render_block`]。块类型（[`BlockKind`]）和是否流式
//! （[`Streaming`]）都由调用方显式声明 —— 渲染方不猜内容是什么，也不猜
//! 内容还在不在蹦字。收「块类型 + 文本 + 流式标志」，返回这块的行。

pub mod blocks;
pub mod cards;
pub mod chat;
pub mod glyphs;
pub mod highlight;
pub mod markdown;
pub mod system;
pub mod theme;
pub mod todo;
pub mod tools;

use ratatui::style::Modifier;
use ratatui::text::Line;

use self::theme::{HistoryTheme, Token};

/// 这块内容按哪种方式画。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// 用户消息：true-black 卡片、白字、强调色竖栏。永远是成品。
    User,
    /// AI 思考：正文同款 markdown，整块压成灰色斜体。
    Reasoning,
    /// AI 正文：plain markdown。
    Assistant,
}

/// 这块内容还在不在流。
///
/// 显式声明，不由渲染方从内容猜：
/// - [`Final`](Streaming::Final) —— 已完整，**可以**进缓存；
/// - [`Live`](Streaming::Live) —— 还在蹦字，**绝不**进缓存（流式内容不
///   进 transcript，缓存那一路根本看不到它；这是数据侧的保证，不是这里
///   的开关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Streaming {
    Final,
    Live,
}

/// 画一块。
///
/// 唯一的渲染入口：`kind` 定画法，`content` 是这块的全文，`streaming`
/// 声明它是否还在流。`width` / `t` 是每帧参数 —— 宽度每帧递入、不存字段，
/// resize 自然重排；主题取当帧的快照。
/// 画一块。
///
/// `defer` = **推迟上色**：代码围栏先按纯文本出图，并把这些段的位置一并
/// 交回来，由缓存决定什么时候补色（见 `blocks::Deferred`）。上色只改样式、
/// 不改行数与文本，所以推迟与不推迟的排版逐行相同——这正是块缓存敢在
/// 首帧只出纯文本的原因。
pub fn render_block(
    kind: BlockKind,
    content: &str,
    streaming: Streaming,
    width: usize,
    t: &HistoryTheme,
    defer: bool,
) -> (Vec<Line<'static>>, Vec<blocks::Deferred>) {
    match kind {
        BlockKind::User => {
            debug_assert_eq!(
                streaming,
                Streaming::Final,
                "用户消息永远是成品，不存在流式"
            );
            (cards::user_card(content, t, width), Vec::new())
        }
        // 流式与非流式在这里画法相同：markdown 直接吃「还没写完」的文本，
        // 未闭合的围栏照常画。两者的区别只在缓存策略（Live 不进缓存）。
        BlockKind::Assistant => markdown::render_markdown(content, t, defer),
        // 思考链的颜色**全部**被下面的灰色斜体覆盖掉：在这里高亮是白烧的钱。
        // 所以它永远按"推迟"渲染，而且不需要谁来补色。
        BlockKind::Reasoning => {
            let (rows, _) = markdown::render_markdown(content, t, true);
            (reasoning_fold(rows, t), Vec::new())
        }
    }
}

/// 思考链：正文 markdown，整块压成系统灰 + 斜体。
fn reasoning_fold(rows: Vec<Line<'static>>, t: &HistoryTheme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for mut line in rows {
        for sp in &mut line.spans {
            sp.style = sp
                .style
                .fg(t.get(Token::SystemText))
                .add_modifier(Modifier::ITALIC);
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // 三种块各画各的：块类型是显式声明，不是从内容猜的。
    #[test]
    fn each_kind_has_its_own_look() {
        let t = t();
        let (user, _) = render_block(BlockKind::User, "问题", Streaming::Final, 20, &t, false);
        let (reason, _) = render_block(BlockKind::Reasoning, "想一下", Streaming::Final, 20, &t, false);
        let (answer, _) = render_block(BlockKind::Assistant, "答案", Streaming::Final, 20, &t, false);

        // 用户消息：卡片，整块黑底。
        assert_eq!(
            user[0].spans[0].style.bg,
            Some(t.get(Token::UserCardBg))
        );
        assert!(text_of(&user).contains("问题"));

        // 思考：灰色斜体。
        let r = reason
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("想一下"))
            .expect("思考文本");
        assert_eq!(r.style.fg, Some(t.get(Token::SystemText)));
        assert!(r.style.add_modifier.contains(Modifier::ITALIC));

        // 正文：不带卡片底、不斜体。
        let a = answer
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains("答案"))
            .expect("正文文本");
        assert_eq!(a.style.bg, None);
        assert!(!a.style.add_modifier.contains(Modifier::ITALIC));
    }

    // 流式与非流式画法相同，但 Live 必须真的把「已收到的那半截」画出来 ——
    // 不能因为还没有流式数据源就吐空。
    #[test]
    fn live_renders_the_text_it_has_so_far() {
        let t = t();
        let partial = "答案的前半";
        let (live, _) = render_block(BlockKind::Assistant, partial, Streaming::Live, 40, &t, false);
        let (done, _) = render_block(BlockKind::Assistant, partial, Streaming::Final, 40, &t, false);
        assert_eq!(text_of(&live), text_of(&done));
        assert!(text_of(&live).contains(partial));
    }
}
