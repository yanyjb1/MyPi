//! Render-performance harness. Exercises the real pipeline (Store load →
//! BlockCache sync → windowed rows_for) against the bench DB, without a
//! terminal or a browser.
//!
//! Scenarios:
//!   cold      — open DB, load 8000 entries, first full-viewport render
//!   scroll    — page-up through the whole transcript (cache miss path)
//!   typing    — per-keystroke frame cost at the bottom of a huge transcript
//!   tools     — heaviest tool-card burst, cold render
//!   mem       — loaded footprint + cache occupancy
//!
//! Usage: cargo run --release --example render_bench [-- scenario]

use std::time::Instant;

use mypi::entry::Entry;
use mypi::tui::bench;

const DB: &str = "/tmp/mypi-bench/sessions.db";
const WIDTH: usize = 120;

fn load_entries() -> Vec<Entry> {
    let store = mypi::store::Store::open(std::path::Path::new(DB)).expect("open bench db");
    let t = Instant::now();
    let entries = store.load_entries(1).expect("load");
    println!(
        "load_entries: {:>8.2} ms  ({} entries)",
        t.elapsed().as_secs_f64() * 1e3,
        entries.len()
    );
    entries
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let only = args.get(1).map(String::as_str);

    mypi::tui::theme::init(None);

    // ---- cold: open + load + first render ----
    if only.is_none() || only == Some("cold") {
        let t0 = Instant::now();
        let entries = load_entries();
        let t_load = t0.elapsed();

        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        let t1 = Instant::now();
        // First frame: bottom window (what a launched TUI paints).
        let (rows, _, _) = bench::window(&mut cache, &entries, n.saturating_sub(8), n, WIDTH);
        let t_first = t1.elapsed();
        println!(
            "cold:        load {:>7.2} ms + first-frame {:>7.2} ms  ({} blocks, {} rows painted)",
            t_load.as_secs_f64() * 1e3,
            t_first.as_secs_f64() * 1e3,
            n,
            rows.len()
        );
    }

    // ---- scroll: page-up from bottom to top, then warm repeat ----
    if only.is_none() || only == Some("scroll") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        // Prime the roster heights (the real app does this on frame 1).
        let _ = bench::window(&mut cache, &entries, n.saturating_sub(8), n, WIDTH);

        // Page-up in ~viewport-sized steps, bottom → top.
        let page_blocks = 12;
        let mut steps: Vec<(usize, usize)> = Vec::new();
        let mut top = n;
        while top > 0 {
            let b0 = top.saturating_sub(page_blocks);
            steps.push((b0, top));
            top = b0;
        }
        let t2 = Instant::now();
        let mut worst = std::time::Duration::ZERO;
        let mut worst_at = 0usize;
        for (i, &(b0, b1)) in steps.iter().enumerate() {
            let t = Instant::now();
            let (rows, _, _) = bench::window(&mut cache, &entries, b0, b1, WIDTH);
            let d = t.elapsed();
            if d > worst {
                worst = d;
                worst_at = b0;
            }
            let _ = rows.len();
            if i % 100 == 0 && i > 0 {
                eprintln!("  scroll step {i}/{}: {}..{} {:.2} ms", i, b0, b1, d.as_secs_f64() * 1e3);
            }
        }
        let total = t2.elapsed();
        println!(
            "scroll-cold: {} steps, total {:>8.1} ms, mean {:>6.2} ms, worst {:>6.2} ms @ block {}",
            steps.len(),
            total.as_secs_f64() * 1e3,
            total.as_secs_f64() * 1e3 / steps.len() as f64,
            worst.as_secs_f64() * 1e3,
            worst_at
        );

        // Warm pass: mostly cache hits (budget evicts, so tail is cold again).
        let t3 = Instant::now();
        for &(b0, b1) in steps.iter().rev() {
            let (rows, _, _) = bench::window(&mut cache, &entries, b0, b1, WIDTH);
            let _ = rows.len();
        }
        let warm = t3.elapsed();
        println!(
            "scroll-warm: {} steps, total {:>8.1} ms, mean {:>6.2} ms",
            steps.len(),
            warm.as_secs_f64() * 1e3,
            warm.as_secs_f64() * 1e3 / steps.len() as f64
        );
    }

    // ---- typing: per-keystroke frame cost at the bottom of a huge transcript ----
    if only.is_none() || only == Some("typing") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        let _ = bench::window(&mut cache, &entries, n.saturating_sub(8), n, WIDTH);

        // A keystroke re-renders the bottom window (cache hits). The input
        // line itself is one Paragraph row — negligible vs history.
        let frames = 200;
        let t4 = Instant::now();
        for k in 0..frames {
            let buf = format!("打字测试 {}", k % 10);
            let _ = buf.len();
            let (rows, _, _) = bench::window(&mut cache, &entries, n.saturating_sub(8), n, WIDTH);
            let _ = rows.len();
        }
        let d = t4.elapsed();
        println!(
            "typing:      {} keystrokes, mean {:>5.2} ms/keystroke (bottom window, cache-hot)",
            frames,
            d.as_secs_f64() * 1e3 / frames as f64
        );
    }

    // ---- tools: heaviest card burst, cold ----
    if only.is_none() || only == Some("tools") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        let _ = bench::window(&mut cache, &entries, n.saturating_sub(8), n, WIDTH);

        // Locate ~a run with many tool results in the middle of the transcript.
        let mut found = None;
        for start in 0..entries.len().saturating_sub(120) {
            let window = &entries[start..start + 120];
            if window.iter().filter(|e| matches!(e, Entry::ToolResult { .. })).count() > 16 {
                found = Some(start);
                break;
            }
        }
        let Some(start) = found else {
            println!("tools: no dense tool region found");
            return;
        };
        let ranges = bench::blocks(&entries);
        let bidx = ranges.iter().position(|r| r.start >= start).unwrap_or(0);
        let t5 = Instant::now();
        let (rows, _, _) = bench::window(&mut cache, &entries, bidx, (bidx + 30).min(n), WIDTH);
        let d = t5.elapsed();
        println!(
            "tools:       30-block tool-dense window render {:>6.2} ms ({} rows)",
            d.as_secs_f64() * 1e3,
            rows.len()
        );
    }

    // ---- memory footprint of the loaded transcript ----
    if only.is_none() || only == Some("mem") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        let (_, cached_rows, cached_blocks) =
            bench::window(&mut cache, &entries, 0, n, WIDTH);
        println!(
            "mem:         {} entries, {} blocks, cache rows={} (budget 8192), cached blocks={}",
            entries.len(),
            n,
            cached_rows,
            cached_blocks
        );
    }
}
