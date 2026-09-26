//! TUI scale benchmark — synthetic transcripts at 8k/12k/16k/24k/32k entries
//! driven through the **real** pipeline: `App` → `MainZone` → 子区 → ratatui
//! buffer diff → ANSI encode. No terminal is opened; the backend writes into
//! a counting sink, so the cost measured is the cost a real frame pays minus
//! the syscall.
//!
//! Scenarios per scale:
//!   ingest    — transcript arriving (attach snapshot) + first full frame
//!   scroll60  — 600 frames of 60 fps input: wheel random walk through the
//!               whole document, random Ctrl+T / Ctrl+O (fold/expand toggles),
//!               random typing. Per-frame latency percentiles.
//!   memory    — allocator live/peak bytes + RSS at each stage
//!
//! Subcommand `gen <scale> <db>` writes the same synthetic transcript through
//! the real `Store` API, so the DB is byte-faithful for the pty/daemon runs.
//!
//! Usage:
//!   cargo run --release --example tui_scale_bench -- 8000 12000 16000 24000 32000
//!   cargo run --release --example tui_scale_bench -- gen 32000 /tmp/x/sessions.db

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use mypi::server::entry::{Align, Entry, TodoPhase, TodoStatus, TodoTask, UsageSummary};
use mypi::tui::app::App;
use mypi::tui::zone::{RawEvent, TermSize, Zone};

// ---------------------------------------------------------------------------
// Counting allocator: the only honest way to attribute "memory vs context".
// ---------------------------------------------------------------------------

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            let n = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, new) };
        if !q.is_null() {
            // New block is live, old one is not: add `new`, give back `l`.
            LIVE.fetch_add(new, Ordering::Relaxed);
            let live = LIVE.fetch_sub(l.size(), Ordering::Relaxed) - l.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}
fn peak_bytes() -> usize {
    PEAK.load(Ordering::Relaxed)
}
fn reset_peak() {
    PEAK.store(live_bytes(), Ordering::Relaxed);
}

/// VmRSS / VmHWM from /proc/self/status (kB).
fn rss_kb() -> (usize, usize) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let get = |k: &str| {
        s.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0)
    };
    (get("VmRSS:"), get("VmHWM:"))
}

// ---------------------------------------------------------------------------
// Synthetic transcript
// ---------------------------------------------------------------------------

/// Deterministic RNG (xorshift64*) — no dependency, reproducible across runs.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn range(&mut self, a: usize, b: usize) -> usize {
        a + self.below(b - a + 1)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const WORDS: &[&str] = &[
    "渲染", "主题", "缓存", "token", "帧率", "滚动", "高亮", "列表", "边框", "diff", "内存",
    "epoch", "syntect", "markdown", "palette", "transcript", "viewport", "budget", "evict",
    "LRU", "row", "spans", "gutter", "italic", "fenced", "inline", "quote", "heading", "cache",
    "render", "layout", "widget", "span", "line", "style", "buffer", "backend", "terminal",
];
const TOOLS: &[&str] = &["bash", "read", "edit", "write", "grep", "glob", "fetch", "search"];

fn lorem(r: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| *r.pick(WORDS))
        .collect::<Vec<_>>()
        .join(" ")
}

fn prose(r: &mut Rng, paras: usize, lo: usize, hi: usize) -> String {
    let mut out = Vec::with_capacity(paras);
    for _ in 0..paras {
        let n = r.range(lo, hi);
        out.push(lorem(r, n));
    }
    out.join("\n\n")
}

/// A realistic assistant turn: headings, list, quote, fenced code.
fn markdown(r: &mut Rng, i: usize) -> String {
    let mut s = String::new();
    s.push_str(&format!("## 第 {i} 节：{}\n\n", lorem(r, 4)));
    s.push_str(&prose(r, 4, 20, 55));
    s.push_str(&format!(
        "\n\n1. {}\n2. {}\n3. {}\n",
        lorem(r, 7),
        lorem(r, 8),
        lorem(r, 6)
    ));
    s.push_str(&format!(
        "\n- {}\n- `{}` 内联代码 {}\n",
        lorem(r, 5),
        r.pick(WORDS),
        lorem(r, 4)
    ));
    s.push_str(&format!("\n> {}\n", lorem(r, 12)));
    s.push_str("\n```rust\n");
    s.push_str(&format!("fn demo_{i}(seed: u64) -> u64 {{\n"));
    s.push_str("    let mut h = 5381u64;\n    for b in seed.to_le_bytes() {\n        h = h.wrapping_mul(33).wrapping_add(b as u64);\n    }\n");
    s.push_str(&format!("    h // {}\n}}\n```", lorem(r, 3)));
    s
}

/// "很脏很脏"的内容：折行、宽度计算、高亮的最坏情况。真实转录里这些都会出现
/// （贴进来的 base64、没空格的 CJK、带 ANSI 的命令输出、emoji、超长单行），
/// 只是不会全挤在一起——冷启动第一帧要量的就是这种最坏情况。
fn dirty_text(r: &mut Rng, lines: usize) -> String {
    let mut out = String::new();
    // 每一块都有：不可断 token、制表符、ANSI、emoji、组合符、零宽字符。
    out.push_str("不可断 token：");
    out.push_str(&"QWxhZGRpbjpvcGVuIHNlc2FtZQ".repeat(20)); // ~520 字符无空格
    out.push_str("\n制表与 ANSI：\t列一\t列二\x1b[31m红\x1b[0m\x1b[1;33m黄\x1b[0m\n");
    out.push_str("emoji 与组合符：👨‍👩‍👧‍👦 🏳️‍🌈 éé́ ‍ 零宽\u{200b}空格\n");
    out.push_str("```rust\nfn 未闭合围栏() {\n");
    // 每 16 块来一次真正的极端：无空格 CJK 长段 + 10k 单行（贴日志/贴 base64）。
    if r.below(16) == 0 {
        out.push_str("无空格 CJK：");
        for _ in 0..200 {
            out.push_str("汉字连绵不绝");
        }
        out.push_str("\n超长单行（10k）：");
        while out.len() < 10240 {
            out.push('x');
        }
        out.push('\n');
    }
    for _ in 0..lines {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            lorem(r, 2),
            "字".repeat(r.range(1, 30)),
            "z".repeat(r.range(1, 90))
        ));
    }
    out.push_str("> 引用里再来一段 ");
    out.push_str(&lorem(r, 20));
    out
}

fn tool_output(r: &mut Rng, lines: usize) -> String {
    (0..lines)
        .map(|k| format!("{:>5}  {}  {:08x}", k, lorem(r, 3), r.next() as u32))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ~1/3 of tool outputs blew past the spill threshold and were replaced by an
/// artifact placeholder — that placeholder, not the full text, is what the
/// renderer chews.
fn artifact_placeholder(r: &mut Rng, id: usize, tool: &str, lines: usize) -> String {
    format!(
        "[工具输出共 {lines} 行 / {}KB，过大已存为巨物 #{id}（{tool}）。\
取用：#{id}（等价文件内容，参与管道：#{id} | grep 关键词 | head -50；裸 #{id} 无过滤会再次巨物化）。前 3 行：\n{}",
        lines * 60 / 1024,
        (0..3)
            .map(|k| format!("{k}  {}", lorem(r, 4)))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// 用户口径的规模是**块**（一条消息、一条回复、一次工具往返），不是条目、
/// 更不是行数。先按经验比例生成，再用真实的 `blocks()` 校正到正好 target 块
/// （截到块边界，免得最后一块半截）。
fn gen_for_blocks(target: usize, seed: u64, dirty: bool) -> Vec<Entry> {
    use mypi::tui::bench::block_count;
    let mut es = gen_entries(target * 3 / 2 + 64, seed, dirty);
    for _ in 0..8 {
        let n = block_count(&es);
        if n == target {
            return es;
        }
        if n > target {
            let ranges = mypi::tui::bench::blocks(&es);
            es.truncate(ranges[target - 1].end);
            return es;
        }
        let mut more = gen_entries((target - n) * 2 + 32, seed ^ 0x51ED, dirty);
        es.append(&mut more);
    }
    es
}

/// One synthetic session. `scale` = entry count; content sizes scale so the
/// byte volume stays proportional (a real session's per-entry size does not
/// shrink as the session grows).
fn gen_entries(scale: usize, seed: u64, dirty: bool) -> Vec<Entry> {
    let mut r = Rng::new(seed);
    let mut out = Vec::with_capacity(scale);
    let mut call = 0usize;
    let mut aid = 0usize;
    while out.len() < scale {
        // Random mix, fixed weights (the user asked for random proportions).
        let roll = r.below(100);
        match roll {
            0..=21 => out.push(Entry::User {
                content: if dirty {
                    dirty_text(&mut r, 3)
                } else {
                    prose(&mut r, 3, 20, 70)
                },
            }),
            22..=45 => {
                if r.below(100) < 45 {
                    out.push(Entry::Reasoning {
                        content: prose(&mut r, 4, 25, 80),
                    });
                    if out.len() >= scale {
                        break;
                    }
                }
                let md = r.below(100) < 25;
                out.push(Entry::Assistant {
                    content: if dirty {
                        dirty_text(&mut r, 6)
                    } else if md {
                        markdown(&mut r, out.len())
                    } else {
                        prose(&mut r, 5, 30, 90)
                    },
                    usage: Some(UsageSummary {
                        total_tokens: r.range(500, 4000) as u64,
                        prompt_tokens: r.range(400, 3500) as u64,
                        cached_tokens: r.range(0, 2000) as u64,
                        completion_tokens: r.range(80, 900) as u64,
                        reasoning_tokens: r.range(0, 400) as u64,
                    }),
                });
            }
            46..=59 => {
                let name = *r.pick(TOOLS);
                call += 1;
                out.push(Entry::ToolRequest {
                    call_id: format!("call_{call}"),
                    name: name.to_string(),
                    args: {
                        let a = lorem(&mut r, 3).replace(' ', "_");
                        format!("{{\"command\": \"{name} {a}\"}}")
                    },
                    intent: lorem(&mut r, 5),
                    text: if r.below(4) == 0 {
                        lorem(&mut r, 6)
                    } else {
                        String::new()
                    },
                    first: true,
                });
                if out.len() >= scale {
                    break;
                }
                let spill = r.below(100) < 33 || name == "bash";
                let lines = if spill {
                    r.range(600, 3000)
                } else {
                    r.range(10, 200)
                };
                let result = if dirty {
                    dirty_text(&mut r, lines.min(40))
                } else if spill {
                    aid += 1;
                    artifact_placeholder(&mut r, aid, name, lines)
                } else {
                    tool_output(&mut r, lines)
                };
                out.push(Entry::ToolResult {
                    call_id: format!("call_{call}"),
                    name: name.to_string(),
                    ok: r.below(100) > 12,
                    result,
                    details: None,
                    duration_ms: 0,
                });
            }
            60..=93 => {
                // A burst of consecutive tool calls, the way a real agent turn
                // behaves (several calls before the next assistant message).
                for _ in 0..r.range(1, 3) {
                    if out.len() >= scale {
                        break;
                    }
                    let name = *r.pick(TOOLS);
                    call += 1;
                    out.push(Entry::ToolRequest {
                        call_id: format!("call_{call}"),
                        name: name.to_string(),
                        args: format!("{{\"path\": \"src/{}.rs\"}}", r.pick(WORDS)),
                        intent: lorem(&mut r, 4),
                        text: String::new(),
                        first: true,
                    });
                    if out.len() >= scale {
                        break;
                    }
                    let spill = r.below(100) < 40;
                    let lines = if spill {
                        r.range(400, 2500)
                    } else {
                        r.range(8, 150)
                    };
                    let result = if spill {
                        aid += 1;
                        artifact_placeholder(&mut r, aid, name, lines)
                    } else {
                        tool_output(&mut r, lines)
                    };
                    out.push(Entry::ToolResult {
                        call_id: format!("call_{call}"),
                        name: name.to_string(),
                        ok: r.below(100) > 10,
                        result,
                        details: None,
                        duration_ms: 0,
                    });
                }
            }
            // todo 随机穿插：条数、状态都随机变——清单贴在历史区底部，
            // 它的高度直接改变聊天窗口的可用行数（用户要量的就是这个）。
            94..=97 => out.push(Entry::Todo {
                phases: (0..r.range(1, 3))
                    .map(|pi| TodoPhase {
                        name: format!("阶段 {pi}"),
                        tasks: (0..r.range(1, 7))
                            .map(|_ti| TodoTask {
                                content: lorem(&mut r, 4),
                                status: match r.below(5) {
                                    0 => TodoStatus::Done,
                                    1 => TodoStatus::InProgress,
                                    2 => TodoStatus::Blocked,
                                    3 => TodoStatus::Abandoned,
                                    _ => TodoStatus::Pending,
                                },
                                blocker: None,
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            98 => out.push(Entry::System {
                text: lorem(&mut r, 6),
                align: if r.below(2) == 0 {
                    Align::Center
                } else {
                    Align::Left
                },
                pin: false,
            }),
            _ => out.push(Entry::Error {
                text: format!("error[{}]: {}", out.len(), lorem(&mut r, 7)),
            }),
        }
    }
    out.truncate(scale);
    out
}

// ---------------------------------------------------------------------------
// Frame driver — the same calls `draw_frame` in session/loop.rs makes.
// ---------------------------------------------------------------------------

/// Terminal backend that counts bytes instead of writing them: the ANSI
/// encode still happens, the syscall does not.
static ANSI_BYTES: AtomicUsize = AtomicUsize::new(0);

struct Sink;
impl std::io::Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        ANSI_BYTES.fetch_add(b.len(), Ordering::Relaxed);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

type Term = ratatui::Terminal<ratatui::backend::CrosstermBackend<Sink>>;

/// `stream` subcommand: what a live streaming tail costs per frame, and — the
/// point of the exercise — what it costs **while it is off screen**.
///
/// Three phases over one transcript, identical except for where the tail is:
///
///   `off`       no tail at all, scrolled up 200 rows (the steady state)
///   `scrolled`  tail growing, scrolled up 200 rows (it is below the viewport)
///   `pinned`    tail growing, following the bottom (it is on screen)
///
/// The tail's text is re-sent every frame (as the daemon does, ~30 Hz) so the
/// memo cannot hide the render: its content changes, its length does not.
fn run_stream(scale: usize, reply_chars: usize, cols: u16, rows: u16, frames: usize, seed: u64) {
    let size = TermSize { cols, rows };
    let entries = gen_entries(scale, seed, false);
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink))
        .expect("terminal");
    let mut app = App::new(size, mypi::tui::zone::ZoneId::Main);
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
    let mut view = mypi::tui::session::view::MainSessionView::new();
    mypi::tui::session::view::dispatch(
        mypi::tui::bench::transcript_of(entries),
        &mut view.zone_for(&mut app.main),
    );
    app.main.settle();

    let reply = reply_text(reply_chars);
    println!(
        "\n=== streaming tail  ({scale} entries, {reply_chars}-char reply, {cols}x{rows}, {frames} frames) ==="
    );

    let mut means: Vec<f64> = Vec::new();
    for phase in ["off", "scrolled", "pinned"] {
        app.main.history.scroll_pinned = phase == "pinned";
        if phase == "scrolled" {
            // 离开底部：往上推一段（渲染帧里按块高兑现）。
            app.main.history.wheel_step(true, 200);
        }
        let mut samples: Vec<u64> = Vec::with_capacity(frames);
        for f in 0..frames {
            if phase != "off" {
                // What the daemon pushes every ~33 ms: the whole buffer,
                // freshly built. Same length every frame (so the tail's height
                // is stable), different content (so the memo cannot hide the
                // render).
                let mut t = reply.clone();
                t.push_str(&f.to_string());
                app.main.history.set_live(String::new(), String::new(), t);
            }
            samples.push(draw(&mut term, &mut app));
        }
        samples.sort_unstable();
        let mean = samples.iter().sum::<u64>() as f64 / frames as f64 / 1e6;
        means.push(mean);
        println!(
            "  [{phase:<8}] mean {mean:>5.2}  p50 {:>5.2}  p99 {:>5.2}  max {:>6.2} ms",
            pct(&samples, 0.50),
            pct(&samples, 0.99),
            samples[samples.len() - 1] as f64 / 1e6
        );
    }
    println!(
        "           off-screen tail {:+.2} ms/frame, on-screen tail {:+.2} ms/frame",
        means[1] - means[0],
        means[2] - means[0]
    );
    println!(
        "           cache {} blocks / {} rows (budget {})",
        app.main.history.cache_stats().0,
        app.main.history.cache_stats().1,
        app.main.history.block_budget(),
    );
}

/// A reply that looks like the real thing: prose + a fenced code block (so the
/// highlight path is exercised), padded to `n` characters.
fn reply_text(n: usize) -> String {
    let mut s = String::with_capacity(n + 128);
    s.push_str("先说结论：尾巴走和成品同一条渲染路径，定稿那一下才不跳。\n\n```rust\n");
    s.push_str(
        "fn main() {\n    let xs: Vec<u32> = (0..10).map(|i| i * 2).collect();\n    println!(\"{xs:?}\");\n}\n",
    );
    s.push_str("```\n\n");
    while s.chars().count() < n {
        s.push_str("理由是折行只做一次，而且未闭合的围栏照常画。\n");
    }
    s
}

/// 一帧的两个半场，分开累计：`ZONE_NS` = 子区出行（我们自己的渲染），
/// `DIFF_NS` = ratatui 的缓冲差分 + ANSI 编码（框架的）。用户问"滚动时历史
/// 在不在反复渲染"，这两个数就是答案的两半。
static ZONE_NS: AtomicUsize = AtomicUsize::new(0);
static DIFF_NS: AtomicUsize = AtomicUsize::new(0);

fn draw(term: &mut Term, app: &mut App) -> u64 {
    let t = Instant::now();
    let mut zone = 0u64;
    term.draw(|f| {
        let t2 = Instant::now();
        let lines = match app.current_owner() {
            mypi::tui::zone::ZoneId::Main => app.main.render(),
            _ => Vec::new(),
        };
        zone = t2.elapsed().as_nanos() as u64;
        f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
        if let Some((x, y)) = app.main.cursor_position() {
            f.set_cursor_position((x, y));
        }
    })
    .expect("draw");
    let total = t.elapsed().as_nanos() as u64;
    ZONE_NS.fetch_add(zone as usize, Ordering::Relaxed);
    DIFF_NS.fetch_add((total - zone) as usize, Ordering::Relaxed);
    total
}


// ---------------------------------------------------------------------------
// 60 fps input profiles
// ---------------------------------------------------------------------------

/// Which input profile a run drives.
#[derive(Clone, Copy, PartialEq)]
enum Profile {
    /// Random mix: wheel random walk, Ctrl-T/Ctrl-O toggles, typing.
    Mixed,
    /// 用户口径的真实滚动：一帧 1~4 个滚轮刻度（3~12 行）＋ 展开/折叠。
    /// 不做大跳——一帧越过几千行是 bench 的随机游走，不是人滚出来的。
    ScrollFold,
    /// Full traversal: wheel from the bottom to the very top, then back down,
    /// with the same random events sprinkled in. **This is the run that fills
    /// the block cache** — the budget is a block *count*, and a random walk
    /// never keeps enough distinct blocks resident to reach it, so the mixed
    /// profile measures a half-empty cache.
    Full,
}

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Profile::Mixed => "mixed",
            Profile::Full => "full ",
            Profile::ScrollFold => "scroll",
        }
    }
}

/// What one 60 fps run measured.
struct SimStats {
    frames: usize,
    samples: Vec<u64>,
    over_budget: usize,
    wheel: usize,
    toggles_t: usize,
    toggles_o: usize,
    typed: usize,
    /// `(nanos, kind, chat_scroll, pinned)` for every frame.
    log: Vec<(u64, &'static str, i64, bool)>,
    wall: std::time::Duration,
    ansi: usize,
    /// Traversals completed (Full profile: bottom → top → bottom).
    laps: usize,
}

/// 视口上沿的可比读数：贴底 = -1，否则按（块键, 行）编一个单调的整数。
fn view_pos(h: &mypi::tui::zone::main::history::HistoryZone) -> i64 {
    match h.top_block() {
        None => -1,
        Some((key, row)) => key.saturating_mul(4096).saturating_add(row as i64),
    }
}

fn simulate(
    term: &mut Term,
    app: &mut App,
    frames: usize,
    profile: Profile,
    seed: u64,
) -> SimStats {
    let mut r = Rng::new(seed);
    let mut samples: Vec<u64> = Vec::with_capacity(frames);
    let mut log: Vec<(u64, &'static str, i64, bool)> = Vec::with_capacity(frames);
    let (mut over_budget, mut wheel, mut toggles_t, mut toggles_o, mut typed) = (0, 0, 0, 0, 0);
    let mut up = true;
    let mut laps = 0usize;
    let bytes0 = ANSI_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut done = 0usize;

    while done < frames {
        let before = view_pos(&app.main.history);
        let mut kind = "scroll";
        match profile {
            Profile::Full => {
                // A page-ish step per frame: a 3-row wheel step would need
                // tens of thousands of frames to cross a 32 000-entry
                // transcript.
                for _ in 0..20 {
                    app.deliver(if up {
                        RawEvent::ScrollUp
                    } else {
                        RawEvent::ScrollDown
                    });
                    wheel += 1;
                }
                // Keep the other input classes in the mix: they are what
                // invalidates cached variants.
                match r.below(100) {
                    0..=5 => {
                        kind = "ctrl-T";
                        app.deliver(RawEvent::Key {
                            key: key(crossterm::event::KeyCode::Char('t'), CONTROL),
                        });
                        toggles_t += 1;
                    }
                    6..=11 => {
                        kind = "ctrl-O";
                        app.deliver(RawEvent::Key {
                            key: key(crossterm::event::KeyCode::Char('o'), CONTROL),
                        });
                        toggles_o += 1;
                    }
                    12..=17 => {
                        kind = "typing";
                        for _ in 0..r.range(1, 3) {
                            app.deliver(RawEvent::Key {
                                key: key(
                                    crossterm::event::KeyCode::Char(*r.pick(&['a', '你', 'x'])),
                                    crossterm::event::KeyModifiers::NONE,
                                ),
                            });
                            typed += 1;
                        }
                    }
                    _ => {}
                }
            }
            Profile::ScrollFold => {
                // 一帧 1~4 个刻度（3~12 行），偶尔按一次展开/折叠。
                let notches = r.range(1, 4);
                for _ in 0..notches {
                    app.deliver(if up {
                        RawEvent::ScrollUp
                    } else {
                        RawEvent::ScrollDown
                    });
                    wheel += 1;
                }
                if r.below(100) < 12 {
                    kind = "ctrl-T";
                    app.deliver(RawEvent::Key {
                        key: key(crossterm::event::KeyCode::Char('t'), CONTROL),
                    });
                    toggles_t += 1;
                } else if r.below(100) < 12 {
                    kind = "ctrl-O";
                    app.deliver(RawEvent::Key {
                        key: key(crossterm::event::KeyCode::Char('o'), CONTROL),
                    });
                    toggles_o += 1;
                }
            }
            Profile::Mixed => {
                let n = r.range(1, 4);
                for _ in 0..n {
                    match r.below(100) {
                        0..=54 => {
                            kind = "scroll";
                            let u = r.below(100) < 62;
                            for _ in 0..r.range(1, 4) {
                                app.deliver(if u {
                                    RawEvent::ScrollUp
                                } else {
                                    RawEvent::ScrollDown
                                });
                                wheel += 1;
                            }
                        }
                        55..=66 => {
                            kind = "ctrl-T";
                            app.deliver(RawEvent::Key {
                                key: key(crossterm::event::KeyCode::Char('t'), CONTROL),
                            });
                            toggles_t += 1;
                        }
                        67..=78 => {
                            kind = "ctrl-O";
                            app.deliver(RawEvent::Key {
                                key: key(crossterm::event::KeyCode::Char('o'), CONTROL),
                            });
                            toggles_o += 1;
                        }
                        79..=97 => {
                            kind = "typing";
                            for _ in 0..r.range(1, 4) {
                                app.deliver(RawEvent::Key {
                                    key: key(
                                        crossterm::event::KeyCode::Char(
                                            *r.pick(&['a', 'b', 'c', '你', '好', 'x', '1']),
                                        ),
                                        crossterm::event::KeyModifiers::NONE,
                                    ),
                                });
                                typed += 1;
                            }
                        }
                        _ => {
                            kind = "backspace";
                            for _ in 0..r.range(1, 3) {
                                app.deliver(RawEvent::Key {
                                    key: key(
                                        crossterm::event::KeyCode::Backspace,
                                        crossterm::event::KeyModifiers::NONE,
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        let d = draw(term, app);
        let after = view_pos(&app.main.history);
        let pinned = app.main.history.scroll_pinned;
        samples.push(d);
        log.push((d, kind, after, pinned));
        if d > 16_666_667 {
            over_budget += 1;
        }
        done += 1;

        if profile == Profile::ScrollFold {
            // 到顶/到底就换向，一直滚（真实用户来回滚）。
            if up && after == before && before > 0 {
                up = false;
            } else if !up && pinned {
                up = true;
            }
        }
        if profile == Profile::Full {
            if up && after == before && before > 0 {
                up = false; // the clamp stopped us: top reached
            } else if !up && pinned {
                laps += 1;
                break; // one lap is the requirement; the cap is only a guard
            }
        }
    }

    SimStats {
        frames: done,
        samples,
        over_budget,
        wheel,
        toggles_t,
        toggles_o,
        typed,
        log,
        wall: started.elapsed(),
        ansi: ANSI_BYTES.load(Ordering::Relaxed) - bytes0,
        laps,
    }
}

fn report(
    profile: Profile,
    st: &SimStats,
    cache: (usize, usize),
    rss: (usize, usize),
    rc: (u64, u64),
    budget: usize,
) {
    let mut sorted = st.samples.clone();
    sorted.sort_unstable();
    let mean = st.samples.iter().sum::<u64>() as f64 / st.frames as f64 / 1e6;
    println!(
        "  [{p}] frames {}  mean {mean:>5.2}  p50 {:>5.2}  p95 {:>5.2}  p99 {:>5.2}  max {:>6.2} ms  over-budget {}/{}",
        st.frames,
        pct(&sorted, 0.50),
        pct(&sorted, 0.95),
        pct(&sorted, 0.99),
        sorted[sorted.len() - 1] as f64 / 1e6,
        st.over_budget,
        st.frames,
        p = profile.name()
    );
    println!(
        "           wall {:.2} s (60 fps budget {:.2} s), {} wheel / {} ctrl-T / {} ctrl-O / {} typed, {} laps, ANSI {:.1} KB/frame",
        st.wall.as_secs_f64(),
        st.frames as f64 / 60.0,
        st.wheel,
        st.toggles_t,
        st.toggles_o,
        st.typed,
        st.laps,
        st.ansi as f64 / 1024.0 / st.frames as f64
    );
    let mut worst = st.log.clone();
    worst.sort_unstable_by_key(|f| std::cmp::Reverse(f.0));
    for (d, kind, scroll, pinned) in worst.iter().take(4) {
        println!(
            "           worst {:>6.2} ms  on {kind:<9} scroll={scroll} pinned={pinned}",
            *d as f64 / 1e6
        );
    }
    let frames = st.frames.max(1) as f64;
    println!(
        "           frame split: zone {:.2} ms + diff/ansi {:.2} ms",
        ZONE_NS.load(Ordering::Relaxed) as f64 / frames / 1e6,
        DIFF_NS.load(Ordering::Relaxed) as f64 / frames / 1e6
    );
    ZONE_NS.store(0, Ordering::Relaxed);
    DIFF_NS.store(0, Ordering::Relaxed);
    println!(
        "           block rows: {} from cache / {} rendered ({} frames)",
        rc.0,
        rc.1,
        st.frames
    );
    println!(
        "           cache {} blocks / {} rows (budget {}); rss {:.1} MB peak {:.1} MB",
        cache.0,
        cache.1,
        budget,
        rss.0 as f64 / 1024.0,
        rss.1 as f64 / 1024.0
    );
}

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i] as f64 / 1e6
}

/// One scale: ingest → cold frame → 600-frame 60 fps simulation.
fn run_scale(blocks_target: usize, cols: u16, rows: u16, frames: usize, seed: u64) {
    println!(
        "\n=== scale {blocks_target} blocks  ({cols}x{rows}, {frames} frames) ==="
    );
    let size = TermSize { cols, rows };

    // ---- ingest: what an attach snapshot costs the front end ----
    reset_peak();
    let t = Instant::now();
    let entries = gen_for_blocks(blocks_target, seed, false);
    let gen_ms = t.elapsed().as_secs_f64() * 1e3;
    let after_gen = live_bytes();
    let payload: usize = entries
        .iter()
        .map(|e| match e {
            Entry::User { content } => content.len(),
            Entry::Assistant { content, .. } => content.len(),
            Entry::Reasoning { content } => content.len(),
            Entry::ToolRequest {
                args, intent, text, ..
            } => args.len() + intent.len() + text.len(),
            Entry::ToolResult { result, .. } => result.len(),
            Entry::Error { text } | Entry::System { text, .. } => text.len(),
            Entry::Name { name } => name.len(),
            Entry::Todo { phases } => phases.len(),
            Entry::Compaction { summary, .. } => summary.len(),
        })
        .sum();

    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink))
        .expect("terminal");
    let mut app = App::new(size, mypi::tui::zone::ZoneId::Main);
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));

    // The exact production path for an attach snapshot: the daemon ships
    // `ServerMsg::Transcript`, the loop hands it to `dispatch`, which moves
    // it into the zone. No copy happens anywhere on that chain.
    // How many blocks the transcript groups into: the cache budget is a
    // block count, so this is the number the budget is compared against.
    let blocks = mypi::tui::bench::block_count(&entries);
    let entries_len = entries.len();
    let mut view = mypi::tui::session::view::MainSessionView::new();
    let t = Instant::now();
    mypi::tui::session::view::dispatch(
        mypi::tui::bench::transcript_of(entries),
        &mut view.zone_for(&mut app.main),
    );
    app.main.settle();
    let ingest_ms = t.elapsed().as_secs_f64() * 1e3;
    let after_ingest = live_bytes();
    let (rss_ingest, _) = rss_kb();

    // Cold frame: first paint of a huge transcript.
    let cold = draw(&mut term, &mut app);
    let after_first = live_bytes();
    let (rss0, _) = rss_kb();
    println!(
        "  blocks {blocks}  entries {}  payload {:.1} MB",
        entries_len,
        payload as f64 / 1e6
    );
    println!("  gen           {gen_ms:>8.1} ms   (synthetic content, not TUI cost)");
    println!("  ingest        {ingest_ms:>8.1} ms   (replace_transcript + settle)");
    println!(
        "  cold frame    {:>8.1} ms   (first paint, empty cache)",
        cold as f64 / 1e6
    );
    println!(
        "  live bytes    gen {:>7.1} MB -> ingest {:>7.1} MB -> first frame {:>7.1} MB",
        after_gen as f64 / 1e6,
        after_ingest as f64 / 1e6,
        after_first as f64 / 1e6
    );
    println!(
        "  rss           {:.1} MB after ingest, {:.1} MB after first frame",
        rss_ingest as f64 / 1024.0,
        rss0 as f64 / 1024.0
    );

    // ---- 冷启动第一帧：脏内容 ----
    // 用户口径："把很脏很脏的消息插进去，冷启动之后的第一帧"。同一块数、内容
    // 换成最坏情况（超长不可断 token、无空格 CJK、制表符、ANSI、emoji、10k 单行、
    // 未闭合围栏），量的是折行/宽度/高亮同时踩满的那一帧。
    {
        reset_peak();
        let dirty_entries = gen_for_blocks(blocks_target, seed ^ 0xD127, true);
        let dirty_blocks = mypi::tui::bench::block_count(&dirty_entries);
        let mut term2 =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink)).expect("term");
        let mut app2 = App::new(size, mypi::tui::zone::ZoneId::Main);
        app2.main
            .reserved
            .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
        let mut view2 = mypi::tui::session::view::MainSessionView::new();
        let t = Instant::now();
        mypi::tui::session::view::dispatch(
            mypi::tui::bench::transcript_of(dirty_entries),
            &mut view2.zone_for(&mut app2.main),
        );
        app2.main.settle();
        let dirty_ingest = t.elapsed().as_secs_f64() * 1e3;
        let dirty_cold = draw(&mut term2, &mut app2);
        let (rss_dirty, hwm_dirty) = rss_kb();
        println!(
            "  dirty cold    {:>8.1} ms   ({dirty_blocks} dirty blocks; ingest {dirty_ingest:.1} ms)",
            dirty_cold as f64 / 1e6
        );
        println!(
            "  dirty rss     {:.1} MB now, {:.1} MB peak",
            rss_dirty as f64 / 1024.0,
            hwm_dirty as f64 / 1024.0
        );
    }

    // ---- 60 fps simulations ----
    // Two profiles per scale: `full` fills the cache (bottom → top → bottom),
    // `mixed` is the random-event walk. The budget is a block count, so only
    // the traversal can show what the cache costs when it is genuinely full.
    let only = std::env::var("BENCH_PROFILE").ok();
    for profile in [Profile::Full, Profile::Mixed, Profile::ScrollFold] {
        // 预算扫描用：只跑一个档，省掉整轮遍历。
        if let Some(o) = &only
            && profile.name().trim() != o.as_str()
        {
            continue;
        }
        // A lap over a 32 000-entry transcript is ~200k rows; at 60 rows per
        // frame that is ~3 300 frames. The cap is a guard, not a target —
        // `full` stops as soon as it has been from the bottom to the top and
        // back, which is the point of the profile.
        let budget = match profile {
            Profile::Full => 30_000,
            Profile::Mixed | Profile::ScrollFold => frames,
        };
        reset_peak();
        let st = simulate(&mut term, &mut app, budget, profile, seed ^ 0x9E37_79B9);
        let (rss1, hwm1) = rss_kb();
        report(
            profile,
            &st,
            app.main.history.cache_stats(),
            (rss1, hwm1),
            app.main.history.render_counts(),
            app.main.history.block_budget(),
        );
        println!(
            "           heap after this profile: live {:.1} MB, peak {:.1} MB",
            live_bytes() as f64 / 1e6,
            peak_bytes() as f64 / 1e6
        );
    }
}

const CONTROL: crossterm::event::KeyModifiers = crossterm::event::KeyModifiers::CONTROL;

fn key(code: crossterm::event::KeyCode, mods: crossterm::event::KeyModifiers) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent {
        code,
        modifiers: mods,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

// ---------------------------------------------------------------------------
// `gen` subcommand — the same transcript through the real Store, for the
// daemon/pty runs.
// ---------------------------------------------------------------------------

fn gen_db(scale: usize, path: &str, dirty: bool) {
    use mypi::server::store::Store;
    let p = std::path::Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    for suf in ["", "-wal", "-shm"] {
        std::fs::remove_file(format!("{path}{suf}")).ok();
    }
    let mut store = Store::open(p).expect("open store");
    let sid = store
        .create_session("2026-09-26 09:00:00", "/tmp/mypi-scale")
        .expect("create session");
    // `scale` 是**块**：和 TUI 侧同一口径。
    let entries = gen_for_blocks(scale, 42, dirty);
    let blocks = mypi::tui::bench::block_count(&entries);
    // Chunked append: one transaction per ~512 entries, the way turns land —
    // but **on block boundaries**, so the stored grouping is the one a real
    // turn would produce (a chunk that split a tool exchange would store two
    // blocks where production stores one).
    let mut batch: Vec<mypi::grouping::Range> = Vec::new();
    let mut batch_len = 0usize;
    for r in mypi::grouping::chunks(&entries) {
        batch_len += r.end - r.start;
        batch.push(r);
        if batch_len >= 512 {
            let start = batch.first().unwrap().start;
            let end = batch.last().unwrap().end;
            store.append(sid, &entries[start..end]).expect("append");
            batch.clear();
            batch_len = 0;
        }
    }
    if let (Some(first), Some(last)) = (batch.first(), batch.last()) {
        store.append(sid, &entries[first.start..last.end]).expect("append");
    }
    let stored = store.stored_blocks(sid).expect("stored blocks").len();
    store.set_session_name(sid, Some(&format!("scale-{scale}"))).ok();
    let n = entries.len();
    let payload: usize = entries
        .iter()
        .map(|e| serde_json::to_string(e).map(|s| s.len()).unwrap_or(0))
        .sum();
    println!(
        "gen: session {sid}, {stored} stored blocks ({blocks} grouped) / {n} entries, ~{:.1} MB payload{}{path}",
        payload as f64 / 1e6,
        if dirty { " (dirty) -> " } else { " -> " }
    );
}

/// `fork <db> <parent> <at-block> [extra-blocks]` — branch a stored session the
/// way a regenerate would, so the daemon/pty runs can be pointed at a session
/// whose transcript spans a fork point.
fn fork_db(path: &str, parent: i64, at: i64, extra: usize) {
    use mypi::server::store::Store;
    let mut store = Store::open(std::path::Path::new(path)).expect("open");
    let id = store.fork_session(parent, at).expect("fork");
    if extra > 0 {
        let entries = gen_for_blocks(extra, 7, false);
        let mut batch: Vec<mypi::grouping::Range> = Vec::new();
        let mut batch_len = 0usize;
        for r in mypi::grouping::chunks(&entries) {
            batch_len += r.end - r.start;
            batch.push(r);
            if batch_len >= 512 {
                let start = batch.first().unwrap().start;
                let end = batch.last().unwrap().end;
                store.append(id, &entries[start..end]).expect("append");
                batch.clear();
                batch_len = 0;
            }
        }
        if let (Some(first), Some(last)) = (batch.first(), batch.last()) {
            store.append(id, &entries[first.start..last.end]).expect("append");
        }
    }
    let own = store.stored_blocks(id).expect("blocks").len();
    let all = store.load_entries(id).expect("branch").len();
    println!("fork: session {id} from {parent} @block {at} -> {own} own blocks, {all} entries on the branch");
}

/// `load <db>` — the server-side cost of attaching: SQLite read path and the
/// context rebuild the hub runs before it can answer `attach`.
/// **用户口径的滚动基准**：真库 + 真窗口 + 真 wire 路径。
///
/// 滚动 = 每次不超过 10 个**块**，随机穿插 Ctrl+O / Ctrl+T（跟人一样随机
/// 展开折叠）。取页那一跳模拟 daemon 的后台线程：**不算进帧时间**，单独报。
fn user_db(path: &str, sid: i64, frames: usize, seed: u64) {
    use mypi::server::store::Store;
    use mypi::server::wire::{ServerMsg, WireBlock};

    let store = Store::open(std::path::Path::new(path)).expect("open");
    let tail_event = Instant::now();
    let (tail, _more) = store.load_tail(sid, 400).expect("load_tail");
    let tail_ms = tail_event.elapsed().as_secs_f64() * 1e3;
    let tail: Vec<WireBlock> = tail
        .into_iter()
        .map(|b| WireBlock {
            id: b.id,
            entries: b.entries,
        })
        .collect();

    let cols: u16 = std::env::var("BENCH_COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let rows: u16 = std::env::var("BENCH_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let size = TermSize { cols, rows };
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink))
        .expect("terminal");
    let mut app = App::new(size, mypi::tui::zone::ZoneId::Main);
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
    let mut view = mypi::tui::session::view::MainSessionView::new();
    mypi::tui::session::view::dispatch(
        ServerMsg::Transcript {
            blocks: tail,
            live: Vec::new(),
        },
        &mut view.zone_for(&mut app.main),
    );
    app.main.settle();
    let cold = draw(&mut term, &mut app);

    let mut r = Rng::new(seed);
    let mut samples: Vec<u64> = Vec::with_capacity(frames);
    let mut fetch_ns: u64 = 0;
    let mut fetches = 0usize;
    let mut up = true;
    let mut laps = 0usize;
    let (mut toggles_t, mut toggles_o) = (0usize, 0usize);
    for _ in 0..frames {
        // 先回答上一帧的开口要数（daemon 那条线程上的事，不计进帧时间）。
        if let Some((newer, edge, count)) = app.main.history.take_want() {
            let t = Instant::now();
            let blocks: Vec<WireBlock> = if newer {
                store
                    .load_after(sid, edge, count)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|b| WireBlock {
                        id: b.id,
                        entries: b.entries,
                    })
                    .collect()
            } else {
                store
                    .load_before(sid, edge, count)
                    .map(|(b, _)| b)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|b| WireBlock {
                        id: b.id,
                        entries: b.entries,
                    })
                    .collect()
            };
            fetch_ns += t.elapsed().as_nanos() as u64;
            fetches += 1;
            let msg = if newer {
                ServerMsg::NewerBlocks { blocks }
            } else {
                ServerMsg::OlderBlocks { blocks }
            };
            mypi::tui::session::view::dispatch(msg, &mut view.zone_for(&mut app.main));
        }
        // 用户的一次滚动：1..=10 块，方向随机翻面（到底/到顶就掉头）。
        let step = r.range(1, 11) as u16;
        app.main.history.scroll_blocks(up, step);
        // 随机穿插展开/折叠（Ctrl+O / Ctrl+T）。
        if r.below(100) < 18 {
            app.deliver(RawEvent::Key {
                key: key(
                    crossterm::event::KeyCode::Char('o'),
                    crossterm::event::KeyModifiers::CONTROL,
                ),
            });
            toggles_o += 1;
        }
        if r.below(100) < 18 {
            app.deliver(RawEvent::Key {
                key: key(
                    crossterm::event::KeyCode::Char('t'),
                    crossterm::event::KeyModifiers::CONTROL,
                ),
            });
            toggles_t += 1;
        }
        let rc0 = app.main.history.render_counts();
        let (z0, a0) = (ZONE_NS.load(Ordering::Relaxed), DIFF_NS.load(Ordering::Relaxed));
        let d = draw(&mut term, &mut app);
        let rc1 = app.main.history.render_counts();
        // BENCH_TRACE=1：把 >30 ms 的帧打印出来（zone 与 ratatui 差分分开），
        // 用来回答"这一帧的时间花在哪"——慢帧几乎都是**第一次排版一张巨型
        // 工具卡**（首页 3000 行），不是窗口本身。
        if std::env::var_os("BENCH_TRACE").is_some() && d > 30_000_000 {
            let (z1, a1) = (ZONE_NS.load(Ordering::Relaxed), DIFF_NS.load(Ordering::Relaxed));
            eprintln!(
                "  slow frame: total {:.1} ms = zone {:.1} + diff/ansi {:.1}  renders {}  窗口 {}",
                d as f64 / 1e6,
                (z1 - z0) as f64 / 1e6,
                (a1 - a0) as f64 / 1e6,
                rc1.1 - rc0.1,
                app.main.history.window_len()
            );
        }
        samples.push(d);
        if app.main.history.scroll_pinned && !up {
            up = true;
            laps += 1;
        } else if !app.main.history.scroll_pinned && up && r.below(100) < 4 {
            up = false;
        }
    }

    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let mean = sorted.iter().sum::<u64>() as f64 / frames as f64 / 1e6;
    let p = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize] as f64 / 1e6;
    let (rss_kb_v, rss_hwm_kb) = rss_kb();
    let peak = peak_bytes() as f64 / 1e6;
    println!(
        "  首帧 {:.2} ms  尾巴装载 {tail_ms:.1} ms  帧 mean {mean:.2} / p50 {:.2} / p95 {:.2} / p99 {:.2} / max {:.2} ms",
        cold as f64 / 1e6,
        p(0.50),
        p(0.95),
        p(0.99),
        sorted[sorted.len() - 1] as f64 / 1e6
    );
    println!(
        "  取页 {fetches} 次 共 {:.1} ms（不计入帧）  翻面 {laps} 次  Ctrl+O {toggles_o} / Ctrl+T {toggles_t}",
        fetch_ns as f64 / 1e6
    );
    println!(
        "  内存 常驻 {:.1} MB（HWM {:.1} MB）  峰值 {peak:.1} MB  窗口 {} 块  行缓存 {} 块 / {} 行",
        rss_kb_v as f64 / 1024.0,
        rss_hwm_kb as f64 / 1024.0,
        app.main.history.window_len(),
        app.main.history.cache_stats().0,
        app.main.history.cache_stats().1,
    );
}

fn load_db(path: &str, sid: i64) {
    use mypi::server::store::Store;
    let p = std::path::Path::new(path);
    let t = Instant::now();
    let store = Store::open(p).expect("open");
    println!("  open           {:>9.1} ms", t.elapsed().as_secs_f64() * 1e3);
    // 冷启动真正走的那几条（第一帧）：尾巴、名字、列表体积、cwd 轨迹——全部
    // O(页)，和会话总长无关。
    let t = Instant::now();
    let (tail, more) = store.load_tail(sid, 400).expect("load_tail");
    let tail_entries: usize = tail.iter().map(|b| b.entries.len()).sum();
    println!(
        "  load_tail(400) {:>9.1} ms  ({} blocks / {tail_entries} entries, more {more})",
        t.elapsed().as_secs_f64() * 1e3,
        tail.len()
    );
    let t = Instant::now();
    let name = store.effective_name(sid).expect("effective_name");
    println!(
        "  effective_name {:>9.1} ms  ({name:?})",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    let sessions = store.list_session_rows(None).expect("list_session_rows");
    println!(
        "  list rows      {:>9.1} ms  ({} sessions)",
        t.elapsed().as_secs_f64() * 1e3,
        sessions.len()
    );
    let t = Instant::now();
    let trail = store.cwd_trail(sid).expect("cwd_trail");
    println!(
        "  cwd_trail      {:>9.1} ms  ({} migrations)",
        t.elapsed().as_secs_f64() * 1e3,
        trail.len()
    );
    let t = Instant::now();
    let entries = store.load_entries(sid).expect("load_entries");
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "  load_entries   {load_ms:>9.1} ms  ({} entries)",
        entries.len()
    );
    let t = Instant::now();
    let ctx = mypi::server::turn::entries_to_context("sys", &entries);
    println!(
        "  entries_to_context {:>5.1} ms  ({} messages)",
        t.elapsed().as_secs_f64() * 1e3,
        ctx.messages.len()
    );
    // Reference only: this is the copy `dispatch` used to make on every
    // attach. It no longer exists on the hot path — kept here so the report
    // can state what removing it was worth.
    let t = Instant::now();
    let cloned = entries.clone();
    println!(
        "  clone (was)    {:>9.1} ms  (the copy dispatch used to make, {} entries)",
        t.elapsed().as_secs_f64() * 1e3,
        cloned.len()
    );
    let t = Instant::now();
    let line = serde_json::to_string(&mypi::tui::bench::transcript_of(entries.clone()));
    let enc_ms = t.elapsed().as_secs_f64() * 1e3;
    let bytes = line.as_ref().map(|l| l.len()).unwrap_or(0);
    println!("  wire encode    {enc_ms:>9.1} ms  ({:.1} MB line)", bytes as f64 / 1e6);
    // 前端把同一份 JSON 再解析一遍的代价（客户端那一半）。
    if let Ok(l) = &line {
        let t = Instant::now();
        let back: mypi::server::wire::ServerMsg = serde_json::from_str(l).expect("decode");
        let dec_ms = t.elapsed().as_secs_f64() * 1e3;
        let n: usize = match back {
            mypi::server::wire::ServerMsg::Transcript { blocks, live } => blocks
                .iter()
                .map(|b| b.entries.len())
                .sum::<usize>()
                + live.len(),
            _ => 0,
        };
        println!("  wire decode    {dec_ms:>9.1} ms  ({n} entries, front-end side)");
    }
    // 归因：`load_entries` 那 260 ms 里，多少是 SQLite 读、多少是 JSON 解析？
    {
        let t = Instant::now();
        let rows = store.bench_raw_rows(sid).expect("raw rows");
        let read_ms = t.elapsed().as_secs_f64() * 1e3;
        let bytes: usize = rows.iter().map(|(_, p)| p.len()).sum();
        let t = Instant::now();
        let mut n = 0usize;
        for (k, p) in &rows {
            if mypi::server::entry::Entry::from_payload(k, p).is_some() {
                n += 1;
            }
        }
        println!(
            "  raw rows       {read_ms:>9.1} ms  ({} rows, {:.1} MB, no parse)",
            rows.len(),
            bytes as f64 / 1e6
        );
        println!(
            "  json parse     {:>9.1} ms  ({n} entries, parse only)",
            t.elapsed().as_secs_f64() * 1e3
        );
    }

    // 分组（`blocks()`）——第一帧 `sync` 里唯一 O(条目) 的活。
    let t = Instant::now();
    let blocks = mypi::tui::bench::block_count(&entries);
    println!(
        "  blocks()       {:>9.1} ms  ({blocks} blocks)",
        t.elapsed().as_secs_f64() * 1e3
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    mypi::tui::theme::init(None);

    if args.first().map(String::as_str) == Some("load") {
        let path = args.get(1).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        let sid: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        println!("=== server attach cost: {path} (session {sid}) ===");
        load_db(&path, sid);
        return;
    }

    if args.first().map(String::as_str) == Some("user") {
        let path = args.get(1).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        let sid: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        let frames: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1500);
        let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0xB10C);
        println!("=== 用户口径滚动：{path}（会话 {sid}，{frames} 帧）===");
        user_db(&path, sid, frames, seed);
        return;
    }

    if args.first().map(String::as_str) == Some("gen") {
        let scale: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8000);
        let path = args.get(2).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        let dirty = args.iter().any(|a| a == "dirty");
        gen_db(scale, &path, dirty);
        return;
    }

    if args.first().map(String::as_str) == Some("fork") {
        let path = args.get(1).cloned().unwrap_or_default();
        let parent: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        let at: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let extra: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        fork_db(&path, parent, at, extra);
        return;
    }

    // One-time cost, paid by whichever code fence renders first: syntect
    // deserializes its whole default syntax set (dozens of languages) into a
    // process-wide static. It is independent of the transcript, so warm it
    // once here, report it, and measure every scale with it already paid —
    // otherwise it shows up as "transcript memory" in whichever scale
    // happened to hit a code fence first.
    {
        let before = live_bytes();
        let theme = mypi::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let _ = mypi::tui::zone::main::history::render::highlight::highlight(
            "fn main() { println!(\"hi\"); }",
            Some("rust"),
            &theme,
        );
        println!(
            "syntect warm-up: +{:.1} MB one-time (syntax set, independent of transcript)\n",
            (live_bytes() - before) as f64 / 1e6
        );
    }

    let stream: Option<usize> = (args.first().map(String::as_str) == Some("stream"))
        .then(|| args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8000));
    let reply: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4000);

    let scales: Vec<usize> = if args.is_empty() || stream.is_some() {
        vec![8000, 12000, 16000, 24000, 32000]
    } else {
        args.iter().filter_map(|s| s.parse().ok()).collect()
    };
    let cols: u16 = std::env::var("BENCH_COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let rows: u16 = std::env::var("BENCH_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let frames: usize = std::env::var("BENCH_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    if let Some(scale) = stream {
        run_stream(scale, reply, cols, rows, frames, 0xC0FFEE ^ scale as u64);
        return;
    }

    for s in scales {
        run_scale(s, cols, rows, frames, 0xC0FFEE ^ s as u64);
    }
}
