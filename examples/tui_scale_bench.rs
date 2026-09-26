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

use mypi::server::entry::{Align, Entry, UsageSummary};
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

/// One synthetic session. `scale` = entry count; content sizes scale so the
/// byte volume stays proportional (a real session's per-entry size does not
/// shrink as the session grows).
fn gen_entries(scale: usize, seed: u64) -> Vec<Entry> {
    let mut r = Rng::new(seed);
    let mut out = Vec::with_capacity(scale);
    let mut call = 0usize;
    let mut aid = 0usize;
    while out.len() < scale {
        // Random mix, fixed weights (the user asked for random proportions).
        let roll = r.below(100);
        match roll {
            0..=21 => out.push(Entry::User {
                content: prose(&mut r, 3, 20, 70),
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
                    content: if md {
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
                let result = if spill {
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
            94..=97 => out.push(Entry::System {
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
    let entries = gen_entries(scale, seed);
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink))
        .expect("terminal");
    let mut app = App::new(size, mypi::tui::zone::ZoneId::Main);
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
    let mut view = mypi::tui::session::view::MainSessionView::new();
    mypi::tui::session::view::dispatch(
        mypi::server::wire::ServerMsg::Transcript { entries },
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
        app.main.history.chat_scroll = if phase == "pinned" { 0 } else { 200 };
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
        mypi::tui::zone::main::history::HistoryZone::block_budget(),
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

fn draw(term: &mut Term, app: &mut App) -> u64 {
    let t = Instant::now();
    term.draw(|f| {
        let lines = match app.current_owner() {
            mypi::tui::zone::ZoneId::Main => app.main.render(),
            _ => Vec::new(),
        };
        f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
        if let Some((x, y)) = app.main.cursor_position() {
            f.set_cursor_position((x, y));
        }
    })
    .expect("draw");
    t.elapsed().as_nanos() as u64
}


// ---------------------------------------------------------------------------
// 60 fps input profiles
// ---------------------------------------------------------------------------

/// Which input profile a run drives.
#[derive(Clone, Copy, PartialEq)]
enum Profile {
    /// Random mix: wheel random walk, Ctrl-T/Ctrl-O toggles, typing.
    Mixed,
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
    log: Vec<(u64, &'static str, usize, bool)>,
    wall: std::time::Duration,
    ansi: usize,
    /// Traversals completed (Full profile: bottom → top → bottom).
    laps: usize,
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
    let mut log: Vec<(u64, &'static str, usize, bool)> = Vec::with_capacity(frames);
    let (mut over_budget, mut wheel, mut toggles_t, mut toggles_o, mut typed) = (0, 0, 0, 0, 0);
    let mut up = true;
    let mut laps = 0usize;
    let bytes0 = ANSI_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut done = 0usize;

    while done < frames {
        let before = app.main.history.chat_scroll;
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
        let after = app.main.history.chat_scroll;
        let pinned = app.main.history.scroll_pinned;
        samples.push(d);
        log.push((d, kind, after, pinned));
        if d > 16_666_667 {
            over_budget += 1;
        }
        done += 1;

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

fn report(profile: Profile, st: &SimStats, cache: (usize, usize), rss: (usize, usize)) {
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
    println!(
        "           cache {} blocks / {} rows (budget {}); rss {:.1} MB peak {:.1} MB",
        cache.0,
        cache.1,
        mypi::tui::zone::main::history::HistoryZone::block_budget(),
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
fn run_scale(scale: usize, cols: u16, rows: u16, frames: usize, seed: u64) {
    println!("\n=== scale {scale} entries  ({cols}x{rows}, {frames} frames) ===");
    let size = TermSize { cols, rows };

    // ---- ingest: what an attach snapshot costs the front end ----
    reset_peak();
    let t = Instant::now();
    let entries = gen_entries(scale, seed);
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
    let mut view = mypi::tui::session::view::MainSessionView::new();
    let t = Instant::now();
    mypi::tui::session::view::dispatch(
        mypi::server::wire::ServerMsg::Transcript { entries },
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
        "  entries {scale}  payload {:.1} MB  blocks {blocks}",
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

    // ---- 60 fps simulations ----
    // Two profiles per scale: `full` fills the cache (bottom → top → bottom),
    // `mixed` is the random-event walk. The budget is a block count, so only
    // the traversal can show what the cache costs when it is genuinely full.
    for profile in [Profile::Full, Profile::Mixed] {
        // A lap over a 32 000-entry transcript is ~200k rows; at 60 rows per
        // frame that is ~3 300 frames. The cap is a guard, not a target —
        // `full` stops as soon as it has been from the bottom to the top and
        // back, which is the point of the profile.
        let budget = match profile {
            Profile::Full => 30_000,
            Profile::Mixed => frames,
        };
        reset_peak();
        let st = simulate(&mut term, &mut app, budget, profile, seed ^ 0x9E37_79B9);
        let (rss1, hwm1) = rss_kb();
        report(profile, &st, app.main.history.cache_stats(), (rss1, hwm1));
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

fn gen_db(scale: usize, path: &str) {
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
    store
        .record_cwd(sid, 0, "/tmp/mypi-scale")
        .expect("record cwd");
    let entries = gen_entries(scale, 42);
    // Chunked append: one transaction per 512 entries, the way rounds land.
    for chunk in entries.chunks(512) {
        store.append(sid, chunk).expect("append");
    }
    store.set_session_name(sid, Some(&format!("scale-{scale}"))).ok();
    let n = entries.len();
    let payload: usize = entries
        .iter()
        .map(|e| serde_json::to_string(e).map(|s| s.len()).unwrap_or(0))
        .sum();
    println!("gen: session {sid}, {n} entries, ~{:.1} MB payload -> {path}", payload as f64 / 1e6);
}

/// `load <db>` — the server-side cost of attaching: SQLite read path and the
/// context rebuild the hub runs before it can answer `attach`.
fn load_db(path: &str) {
    use mypi::server::store::Store;
    let p = std::path::Path::new(path);
    let t = Instant::now();
    let store = Store::open(p).expect("open");
    println!("  open           {:>9.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let entries = store.load_entries(1).expect("load_entries");
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
    let line = serde_json::to_string(&mypi::server::wire::ServerMsg::Transcript { entries });
    println!(
        "  wire encode    {:>9.1} ms  ({:.1} MB line)",
        t.elapsed().as_secs_f64() * 1e3,
        line.map(|l| l.len()).unwrap_or(0) as f64 / 1e6
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    mypi::tui::theme::init(None);

    if args.first().map(String::as_str) == Some("load") {
        let path = args.get(1).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        println!("=== server attach cost: {path} ===");
        load_db(&path);
        return;
    }

    if args.first().map(String::as_str) == Some("gen") {
        let scale: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8000);
        let path = args.get(2).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        gen_db(scale, &path);
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
