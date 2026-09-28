//! 性能基准 —— `mypi bench <场景> [参数…]`。
//!
//! 量的东西按 `ABOUT_TESTS.txt` 的规格来：**MainZone 的渲染性能**（外部网络
//! 的延迟不在这里，也不该在这里）。走的是**真管线**：`App` → `MainZone` →
//! 各子区 → ratatui 的缓冲差分 → ANSI 编码。终端不开；后端把字节写进一个计数
//! 水槽，所以量到的就是真帧的成本减去那次 `write` 系统调用。
//!
//! 为什么是子命令而不是 example：规格里的每一档（8k/12k/24k/32k/64k/128k）
//! 都要反复跑、要对着 CI 和人手比，example 每次都得敲一长串路径。`mypi bench`
//! 是装上的这个二进制本来就有的东西。场景清单见 `mypi bench help`。
//!
//! 内存一律取 `/proc/self/status` 的 VmRSS / VmHWM：内核的读数、零开销、
//! 无侵入。不装计数分配器——那是给这个二进制**所有**进程（包括用户真在用的
//! TUI）每次 malloc 都加一道原子操作，为一份基准污染正常路径不划算。
//!
//! **读数都是增量**：先量一遍合成数据集自己占多少（那是我喂进去的，不是
//! 渲染花的），之后每条内存都减掉它。想看真路径的常驻，看 `mypi bench window`
//! —— 分页取页 + 窗口缓存，那才是用户会遇到的数字。
//!
//! 环境变量：`BENCH_COLS`(120) `BENCH_ROWS`(40) `BENCH_FRAMES`(600)。

const USAGE: &str = "\
mypi bench — render benchmarks (src/bench.rs)

Usage: mypi bench <scenario> [args…]

  scale [blocks…]        synthetic transcripts: ingest + cold frame + 600-frame scroll
                         (bare = the spec sweep 8k/12k/24k/32k/64k/128k; `all` = same)
  gen <blocks> <db> [dirty]   write that transcript through the real Store
  fork <db> <parent> <block> [extra]   branch a stored session (regenerate)
  load <db> [session]    server-side attach cost (SQLite read + JSON parse)
  stream <blocks> [reply-chars]   streaming tail frames, on and off screen
  window <db> <session> [frames] [seed]   real db + real viewport + real wire path
                         (on-demand paging: THIS is the resident-memory picture)

`all` / a bare `mypi bench` also builds a real db and runs `window` on it, because
the `scale` numbers hold the whole transcript in one process by construction.

Env: BENCH_COLS (120) BENCH_ROWS (40) BENCH_FRAMES (600)";

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::server::entry::{Align, Entry, TodoPhase, TodoStatus, TodoTask, UsageSummary};
use crate::tui::app::App;
use crate::tui::zone::{RawEvent, TermSize, Zone};

/// VmRSS / VmHWM from /proc/self/status (kB)。常驻与峰值，内核自己记。
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

/// 常驻内存（字节）。取名沿用原 example：调用点是"这块东西值多少内存"的
/// 归因点，换成 RSS 之后归因变粗（分配器不还给内核的那些算在里面），
/// 但 RSS 才是规格要的"常驻内存"。
fn live_bytes() -> usize {
    rss_kb().0 * 1024
}

/// 本进程烧掉的 CPU 秒数（用户 + 系统），来自 /proc/self/stat。
///
/// 字段是 tick 计数（USER_HZ=100，即 1 秒 100 tick）。`comm` 里可以有空格
/// 和括号，所以先砍到最后一个 `)` 之后，再按字段位取 14/15。
fn cpu_secs() -> f64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = s.rsplit_once(')').map(|(_, r)| r).unwrap_or("");
    let f: Vec<&str> = rest.split_whitespace().collect();
    let t = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    // 砍掉 pid 与 comm 之后 state 落在 0 位：utime(14) → 11，stime(15) → 12。
    (t(11) + t(12)) / 100.0
}

/// 峰值常驻内存（字节）。VmHWM 是内核记的高水位，只有它降不下去。
fn peak_bytes() -> usize {
    rss_kb().1 * 1024
}

/// 无操作：峰值由内核记，没有"重置"这回事。留个名字，免得调用点全要改。
fn reset_peak() {}

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
/// 一行的硬上限（**字符**，按 UTF-8 字节数近似）。
///
/// 现代模型一次性输出的东西不会吐出超长单行：真正的长输出是多行、每行几十
/// 到几百字符。所以这里给每行封顶 500 字节——够宽，能压到折行与宽度计算的
/// 边界；但不再造 10k 单行那种产物。syntect 的单行成本随长度陡增（实测 10k
/// 字符一行 ≈100ms），把它留在基准里只会测一个现实中不存在的形态。
const MAX_LINE_BYTES: usize = 500;

fn dirty_text(r: &mut Rng, lines: usize) -> String {
    let mut out = String::new();
    // 每一块都有：不可断 token、制表符、ANSI、emoji、组合符、零宽字符。
    // 这些都是真实的窄行，保留——它们考的是折行与宽度计算，不是单行长度。
    out.push_str("不可断 token：");
    out.push_str(&"QWxhZGRpbjpvcGVuIHNlc2FtZQ".repeat(6)); // ~156 字符无空格
    out.push_str("\n制表与 ANSI：\t列一\t列二\x1b[31m红\x1b[0m\x1b[1;33m黄\x1b[0m\n");
    out.push_str("emoji 与组合符：👨‍👩‍👧‍👦 🏳️‍🌈 éé́ ‍ 零宽\u{200b}空格\n");
    // 无空格 CJK：真实存在（中文不分词），但封顶在 MAX_LINE_BYTES。
    let mut cjk = String::from("无空格 CJK：");
    while cjk.len() + "汉字连绵不绝".len() <= MAX_LINE_BYTES {
        cjk.push_str("汉字连绵不绝");
    }
    out.push_str(&cjk);
    out.push('\n');
    out.push_str("```rust\nfn 未闭合围栏() {\n");
    for _ in 0..lines {
        // 每行封顶：普通内容 + 一段可长可短的 CJK + 一段 ASCII 尾巴。
        let mut line = format!("{}\t", lorem(r, 2));
        for _ in 0..r.range(1, 4) {
            line.push_str(&"字".repeat(r.range(1, 30)));
        }
        line.push('\t');
        line.push_str(&"z".repeat(r.range(1, 90)));
        if line.len() > MAX_LINE_BYTES {
            line.truncate(MAX_LINE_BYTES);
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("> 引用里再来一段 ");
    out.push_str(&lorem(r, 20));
    out
}

/// 脏内容的行数：真正用上 `lines`，但夹到一个上界。
///
/// 规格关心的是「一屏块 ≤500 行」——超过 500 行的东西会被巨物机制兜底，
/// 不在关心范围。所以这里封在 500，既不丢 `r.range(600, 3000)` 的意图
/// （"这是个溢出级的输出"），也不越过规格划的那条线。
fn dirty_lines(lines: usize) -> usize {
    lines.clamp(10, 500)
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
    use crate::tui::bench::block_count;
    let mut es = gen_entries(target * 3 / 2 + 64, seed, dirty);
    for _ in 0..8 {
        let n = block_count(&es);
        if n == target {
            return es;
        }
        if n > target {
            let ranges = crate::tui::bench::blocks(&es);
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
                    // 走脏内容这条路时 `lines` 就是真行数：以前这里是
                    // `lines.min(40)`，把上面 `r.range(600, 3000)` 整段丢弃，
                    // 于是那个区间成了死参数、脏块永远 ≤150 行——测不到规格
                    // 写的「500 行以内」。现在按真行数生成。
                    dirty_text(&mut r, dirty_lines(lines))
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
    let mut app = App::new(size, crate::tui::zone::ZoneId::Main, &Default::default());
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
    let mut view = crate::tui::session::view::MainSessionView::new();
    crate::tui::session::view::dispatch(
        transcript_owned(entries),
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
            crate::tui::zone::ZoneId::Main => app.main.render(),
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
fn view_pos(h: &crate::tui::zone::main::history::HistoryZone) -> i64 {
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
            "           cache {} blocks / {} rows (budget {}); 滚动净增 +{:.1} MB, 进程高水位 +{:.1} MB (基线=进入滚动前)",
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
    let cpu0 = cpu_secs();
    let t0 = Instant::now();

    let size = TermSize { cols, rows };

    // ---- ingest: what an attach snapshot costs the front end ----
    reset_peak();
    let t = Instant::now();
    let entries = gen_for_blocks(blocks_target, seed, false);
    let gen_ms = t.elapsed().as_secs_f64() * 1e3;
    let after_gen = rss_kb().0;
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
    let mut app = App::new(size, crate::tui::zone::ZoneId::Main, &Default::default());
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));

    // The exact production path for an attach snapshot: the daemon ships
    // `ServerMsg::Transcript`, the loop hands it to `dispatch`, which moves
    // it into the zone. No copy happens anywhere on that chain.
    // How many blocks the transcript groups into: the cache budget is a
    // block count, so this is the number the budget is compared against.
    let blocks = crate::tui::bench::block_count(&entries);
    let entries_len = entries.len();
    let mut view = crate::tui::session::view::MainSessionView::new();
    let t = Instant::now();
    crate::tui::session::view::dispatch(
        transcript_owned(entries),
        &mut view.zone_for(&mut app.main),
    );
    app.main.settle();
    let ingest_ms = t.elapsed().as_secs_f64() * 1e3;
    let after_ingest = rss_kb().0;


    // Cold frame: first paint of a huge transcript.
    let cold = draw(&mut term, &mut app);

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
    // 这一档的基线：合成数据集本身（还没进 TUI）。**下面每一条内存读数都是
    // 相对它的增量** —— 数据集是压测台喂进去的，不该记在渲染头上。
    let base = after_gen;
    println!(
        "  内存基线       {:.1} MB（合成数据集本身；下面的读数都已减掉它）",
        base as f64 / 1024.0
    );
    println!(
        "  ingest 增量   +{:.1} MB",
        (after_ingest.saturating_sub(base)) as f64 / 1024.0
    );
    println!(
        "  首帧后增量    +{:.1} MB   (第一次上屏之后的常驻)",
        (rss0.saturating_sub(base)) as f64 / 1024.0
    );

    // ---- 冷启动第一帧：脏内容 ----
    // 用户口径："把很脏很脏的消息插进去，冷启动之后的第一帧"。同一块数、内容
    // 换成最坏情况（超长不可断 token、无空格 CJK、制表符、ANSI、emoji、10k 单行、
    // 未闭合围栏），量的是折行/宽度/高亮同时踩满的那一帧。
    {
        reset_peak();
        let dirty_entries = gen_for_blocks(blocks_target, seed ^ 0xD127, true);
        let dirty_blocks = crate::tui::bench::block_count(&dirty_entries);
        // 脏内容**自己**占的内存也是基线（第二份数据集），减掉。
        let dirty_base = rss_kb().0;
        let mut term2 =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(Sink)).expect("term");
        let mut app2 = App::new(size, crate::tui::zone::ZoneId::Main, &Default::default());
        app2.main
            .reserved
            .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
        let mut view2 = crate::tui::session::view::MainSessionView::new();
        let t = Instant::now();
        crate::tui::session::view::dispatch(
            transcript_owned(dirty_entries),
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
            "  dirty 增量    +{:.1} MB 常驻, +{:.1} MB 高水位   (脏内容渲染占的)",
            (rss_dirty.saturating_sub(dirty_base)) as f64 / 1024.0,
            (hwm_dirty.saturating_sub(dirty_base)) as f64 / 1024.0
        );
    }

    // ---- 60 fps simulations ----
    // Two profiles per scale: `full` fills the cache (bottom → top → bottom),
    // `mixed` is the random-event walk. The budget is a block count, so only
    // the traversal can show what the cache costs when it is genuinely full.
    let only = std::env::var("BENCH_PROFILE").ok();
    // 滚动段的基线：**进循环的这一刻**。不能拿数据集基线算——脏内容那一段
    // 自己也造了一份 12.8 万块的数据集，drop 之后 glibc 并不还给内核，
    // 那 280 MB 会原样挂在 RSS 上，算进"渲染占用"里就成了一笔冤枉账。
    let p_base = rss_kb().0;
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
            // 相对**滚动段基线**的增量：这才是"这 600 帧让常驻涨了多少"。
            // 绝对值在这里没有意义——它大半是压测台喂进来的转录。
            (rss1.saturating_sub(p_base), hwm1.saturating_sub(p_base)),
            app.main.history.render_counts(),
            app.main.history.block_budget(),
        );
    }
    // CPU：这一段（含三档 profile）烧掉的时间除以墙钟就是平均占用核数。
    // 「常驻 CPU」是它；「峰值」不在这个数里——那是单帧最坏那一跳，
    // p99/max 已经量了它。
    let cpu = cpu_secs() - cpu0;
    let wall = t0.elapsed().as_secs_f64();
    println!("  cpu {cpu:.2} s (user+sys) over {wall:.1} s wall => {:.2} cores", cpu / wall.max(1e-9));
}

/// 把 entries 切成 wire 块，**零复制**。
///
/// lib 里的 `transcript_of` 用 `to_vec` 给每个块做副本——那是给调用方保留原
/// `entries` 的语义。基准里那等于把整份数据集算两遍，内存读数于是混进了
/// 「我们自己又复制了一份」，跟渲染没有半点关系。`grouping::chunks` 给出的
/// 区间递增且不重叠，所以倒着 `split_off` 就能原地切完，一个字节都不多。
fn transcript_owned(entries: Vec<Entry>) -> crate::server::wire::ServerMsg {
    use crate::server::wire::WireBlock;
    let ranges = crate::grouping::chunks(&entries);
    let mut rest = entries;
    let mut slots: Vec<Option<WireBlock>> = (0..ranges.len()).map(|_| None).collect();
    for (i, r) in ranges.iter().enumerate().rev() {
        let tail = rest.split_off(r.start);
        slots[i] = Some(WireBlock {
            id: i as i64 + 1,
            entries: tail,
        });
    }
    crate::server::wire::ServerMsg::Transcript {
        blocks: slots.into_iter().flatten().collect(),
        live: Vec::new(),
    }
}

/// 规格里点名的六档（块）。
const SPEC_SCALES: &[usize] = &[8000, 12000, 24000, 32000, 64000, 128000];

/// `all` / 空参数：**一档一进程**，外加最后那个真库分页的 `window`。
///
/// 这不是洁癖。VmHWM 是**进程级**高水位、只涨不落，glibc 的 arena 也很少
/// 把内存还给内核。同一进程里串跑六档，后面那档读到的是前面的残留：128k 块
/// 那档会报「常驻 609 MB」，而它单独跑只有 36 MB。一个进程只跑一档，
/// 每个数字才真的是那一档自己的。
///
/// `gen` 也单独跑：它要把 240 MB 写进 SQLite，那笔内存在随后的读数里
/// 不该被算作任何东西。
fn run_all_in_children(frames: usize) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let run = |a: &[&str]| -> anyhow::Result<()> {
        let st = std::process::Command::new(&exe).args(["bench"]).args(a).status()?;
        if !st.success() {
            anyhow::bail!("`mypi bench {}` failed", a.join(" "));
        }
        Ok(())
    };
    for s in SPEC_SCALES {
        run(&[&s.to_string()])?;
    }
    let big = SPEC_SCALES.last().copied().unwrap_or(128_000);
    let db = std::env::var("BENCH_DB").unwrap_or_else(|_| {
        std::env::temp_dir()
            .join("mypi-bench-all.db3")
            .to_string_lossy()
            .into_owned()
    });
    let big_s = big.to_string();
    run(&["gen", &big_s, &db])?;
    println!("\n=== window {big} blocks（真库分页取页 —— 规格里的「常驻内存」看这一行）===");
    run(&["window", &db, "1", &frames.to_string()])?;
    Ok(())
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
    use crate::server::store::Store;
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
    let blocks = crate::tui::bench::block_count(&entries);
    // Chunked append: one transaction per ~512 entries, the way turns land —
    // but **on block boundaries**, so the stored grouping is the one a real
    // turn would produce (a chunk that split a tool exchange would store two
    // blocks where production stores one).
    let mut batch: Vec<crate::grouping::Range> = Vec::new();
    let mut batch_len = 0usize;
    for r in crate::grouping::chunks(&entries) {
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
    use crate::server::store::Store;
    let mut store = Store::open(std::path::Path::new(path)).expect("open");
    let id = store.fork_session(parent, at).expect("fork");
    if extra > 0 {
        let entries = gen_for_blocks(extra, 7, false);
        let mut batch: Vec<crate::grouping::Range> = Vec::new();
        let mut batch_len = 0usize;
        for r in crate::grouping::chunks(&entries) {
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
    use crate::server::store::Store;
    use crate::server::wire::{ServerMsg, WireBlock};

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
    let mut app = App::new(size, crate::tui::zone::ZoneId::Main, &Default::default());
    app.main
        .reserved
        .attach_completion(std::path::PathBuf::from("/tmp"), std::path::PathBuf::from("/root"));
    let mut view = crate::tui::session::view::MainSessionView::new();
    crate::tui::session::view::dispatch(
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
            crate::tui::session::view::dispatch(msg, &mut view.zone_for(&mut app.main));
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
    use crate::server::store::Store;
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
    let ctx = crate::server::turn::entries_to_context("sys", &entries);
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
    let line = serde_json::to_string(&transcript_owned(entries.clone()));
    let enc_ms = t.elapsed().as_secs_f64() * 1e3;
    let bytes = line.as_ref().map(|l| l.len()).unwrap_or(0);
    println!("  wire encode    {enc_ms:>9.1} ms  ({:.1} MB line)", bytes as f64 / 1e6);
    // 前端把同一份 JSON 再解析一遍的代价（客户端那一半）。
    if let Ok(l) = &line {
        let t = Instant::now();
        let back: crate::server::wire::ServerMsg = serde_json::from_str(l).expect("decode");
        let dec_ms = t.elapsed().as_secs_f64() * 1e3;
        let n: usize = match back {
            crate::server::wire::ServerMsg::Transcript { blocks, live } => blocks
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
            if crate::server::entry::Entry::from_payload(k, p).is_some() {
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
    let blocks = crate::tui::bench::block_count(&entries);
    println!(
        "  blocks()       {:>9.1} ms  ({blocks} blocks)",
        t.elapsed().as_secs_f64() * 1e3
    );
}

/// `mypi bench <场景> [参数…]` 的入口。参数就是子命令后面那串，原样交给这里。
pub fn run(argv: &[String]) -> anyhow::Result<()> {
    main_impl(argv);
    Ok(())
}

fn main_impl(args: &[String]) {
    if matches!(
        args.first().map(String::as_str),
        Some("help" | "-h" | "--help")
    ) {
        println!("{USAGE}");
        return;
    }
    if args.is_empty() {
        println!("{USAGE}\n");
    }
    crate::tui::theme::init(None);

    if args.first().map(String::as_str) == Some("load") {
        let path = args.get(1).cloned().unwrap_or_else(|| "/tmp/mypi-scale/sessions.db".into());
        let sid: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        println!("=== server attach cost: {path} (session {sid}) ===");
        load_db(&path, sid);
        return;
    }

    if args.first().map(String::as_str) == Some("window") {
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
        let theme = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let _ = crate::tui::zone::main::history::render::highlight::highlight(
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

    let cols: u16 = std::env::var("BENCH_COLS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let rows: u16 = std::env::var("BENCH_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let frames: usize = std::env::var("BENCH_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(600);

    // `all` 是那份规格档位的显式写法（方便人读，也方便脚本）：其余场景都不吃
    // 位置参数，所以"没给档位"和"给了 all"是同一件事。
    let specs: Vec<usize> = match args.first().map(String::as_str) {
        None | Some("all") => {
            // 一档一进程，见 `run_all_in_children`。父进程自己一个像素都不渲染。
            if let Err(e) = run_all_in_children(frames) {
                eprintln!("bench all: {e}");
                std::process::exit(1);
            }
            return;
        }
        _ => args.iter().filter_map(|s| s.parse().ok()).collect(),
    };
    let scales: Vec<usize> = if stream.is_some() { Vec::new() } else { specs };
    if let Some(scale) = stream {
        run_stream(scale, reply, cols, rows, frames, 0xC0FFEE ^ scale as u64);
        return;
    }

    for s in &scales {
        run_scale(*s, cols, rows, frames, 0xC0FFEE ^ *s as u64);
    }

}
