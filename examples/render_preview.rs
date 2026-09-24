//! Render a realistic chat transcript through the real pipeline
//! (markdown + syntect + theme) into a PNG via ratatui's TestBackend.
//!
//! Not a screenshot of a live terminal — but every row is produced by the
//! same code path the TUI runs, so colors/wrapping/cards are exactly what
//! the terminal draws. Only the rasterization is synthetic.

use mypi::entry::{Align, Entry};
use mypi::tui::theme;
use ratatui::backend::TestBackend;
use ratatui::text::Line;
use ratatui::Terminal;

fn main() -> anyhow::Result<()> {
    let entries = vec![
        Entry::Name { name: "重命名触发主题".into() },
        Entry::System {
            text: "已命名：渡鸦（主题 dark）".into(),
            align: Align::Center,
        },
        Entry::User {
            content: "帮我看下这个项目的渲染实现？重点 markdown 列表和代码高亮喵".into(),
        },
        Entry::Assistant {
            reasoning: Some("用户想要聊天区渲染效果，用 1/2/3 级标题、列表、代码块和引用各来一段，覆盖主要 token。".into()),
            content: "\
## 渲染架构

1. **块化渲染**：transcript 按 Node 分块，LRU 缓存行
2. **主题系统**：60 个语义 token，运行时可换
3. **语法高亮**：syntect 解析 + 11 类语义映射

关键点总结：

- 变量 `theme` 是全局单例
- `rotate_for_seed` 用 djb2 散列选主题
- 换主题只 bump 一个 epoch

> 所有着色走 token，没有硬编码的 `Color::Green`。

```rust
pub fn rotate_for_seed(seed: &str) {
    let mut h: u64 = 5381;
    for b in seed.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    let chosen = BUNDLED[h as usize % BUNDLED.len()].0;
    let _ = set_theme(chosen); // 同种子永远同主题
}
```

详见 `docs/omp-theme-port.md`。".into(),
            usage: None,
        },
        Entry::ToolRequest {
            call_id: "c1".into(),
            name: "bash".into(),
            args: r#"{"command":"cargo test --lib | tail -3"}"#.into(),
            intent: "跑测试".into(),
        },
        Entry::ToolResult {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: true,
            result: "test result: ok. 423 passed; 0 failed\nFinished `dev` profile".into(),
        },
        Entry::ToolRequest {
            call_id: "c2".into(),
            name: "edit".into(),
            args: r#"{"path":"src/tui/theme/mod.rs","old":"let palette = Palette::from_config();","new":"let palette = Palette::current();"}"#.into(),
            intent: "改初始化".into(),
        },
        Entry::ToolResult {
            call_id: "c2".into(),
            name: "edit".into(),
            ok: false,
            result: "error[E0308]: mismatched types\n  --> src/tui/session/loop.rs:226:45".into(),
        },
        Entry::System {
            text: "会话已压缩，上下文回收 42k tokens".into(),
            align: Align::Center,
        },
    ];

    // The name marker rotates the theme, mirroring the real /name path.
    theme::init(None);
    for e in &entries {
        if let Entry::Name { name } = e {
            theme::rotate_for_seed(name);
        }
    }

    let width = 100usize;
    let lines: Vec<Line<'static>> = mypi::tui::render_transcript_public(&entries, true, false, width);

    let area_w = width as u16;
    let area_h = (lines.len() as u16).clamp(1, 60);
    let mut term = Terminal::new(TestBackend::new(area_w, area_h))?;
    term.draw(|f| {
        use ratatui::widgets::{Block, Borders, Paragraph};
        let t = theme::theme();
        let block = Block::default().borders(Borders::ALL).border_style(t.fg_style(theme::ColorToken::Border));
        let para = Paragraph::new(lines.clone()).block(block);
        f.render_widget(para, f.area());
    })?;

    // Dump the TestBackend buffer as truecolor HTML, then rasterize to PNG
    // through the shared browser (the project's own screenshot path).
    let b = term.backend().buffer().clone();
    let mut html = String::from("<html><head><meta charset='utf-8'><style>body{margin:0}pre{font:14px/1.45 'Sarasa Mono SC','JetBrains Mono',monospace;margin:0;padding:14px 18px;background:#0f1216;letter-spacing:0}</style></head><body>");
    html.push_str(&format!(
        "<pre style='background:{}'>",
        css(theme::theme().color(theme::ColorToken::UserMessageBg))
    ));
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            let cell = &b[(x, y)];
            let (fg, bg) = (cell.fg, cell.bg);
            let mut mods = String::new();
            if cell.modifier.contains(ratatui::style::Modifier::BOLD) {
                mods.push_str("font-weight:bold;");
            }
            if cell.modifier.contains(ratatui::style::Modifier::ITALIC) {
                mods.push_str("font-style:italic;");
            }
            let sym = cell.symbol();
            let escaped = sym
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let fgc = if fg == ratatui::style::Color::Reset { "#e8ecf4".to_string() } else { css(fg) };
            let bgc = if bg == ratatui::style::Color::Reset { "transparent".to_string() } else { css(bg) };
            html.push_str(&format!("<span style='color:{fgc};background:{bgc};{mods}'>{escaped}</span>"));
        }
        html.push('\n');
    }
    html.push_str("</pre></body></html>");

    let path = std::env::temp_dir().join("mypi-render-preview.html");
    std::fs::write(&path, html)?;
    println!("{}", path.display());
    Ok(())
}

fn css(c: ratatui::style::Color) -> String {
    match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        ratatui::style::Color::Reset => "transparent".into(),
        other => {
            // Best-effort ANSI→hex for the few indexed colors left.
            let _ = other;
            "#e8ecf4".into()
        }
    }
}
