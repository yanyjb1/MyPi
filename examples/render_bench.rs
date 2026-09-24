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
/// `MYPI_BENCH_DB` overrides the DB path (12k/20k scaling runs).
fn db() -> String {
    std::env::var("MYPI_BENCH_DB").unwrap_or_else(|_| DB.to_string())
}
const WIDTH: usize = 120;
const VIEWPORT: usize = 40;

fn load_entries() -> Vec<Entry> {
    let store = mypi::store::Store::open(std::path::Path::new(&db())).expect("open bench db");
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
        let (rows, cached_rows, cached_blocks) =
            bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);
        let t_first = t1.elapsed();
        println!(
            "cold:        load {:>7.2} ms + first-frame {:>7.2} ms  ({} blocks, {} rows painted)",
            t_load.as_secs_f64() * 1e3,
            t_first.as_secs_f64() * 1e3,
            n,
            rows.len()
        );
    }

    // ---- scroll: wheel up from the bottom to the top ----
    if only.is_none() || only == Some("scroll") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let _ = bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);

        // Wheel-up in 3-row steps until the window top hits block 0.
        let mut offset = 0usize;
        let mut steps = 0usize;
        let mut worst = std::time::Duration::ZERO;
        let mut worst_at = 0usize;
        let t2 = Instant::now();
        // Upper bound on total rows: every block renders at most ~200 rows.
        let max_offset = bench::block_count(&entries) * 200;
        loop {
            let t = Instant::now();
            let (b0, _b1) = bench::walk_window(&mut cache, &entries, offset, VIEWPORT, WIDTH);
            let d = t.elapsed();
            if d > worst {
                worst = d;
                worst_at = offset;
            }
            steps += 1;
            if b0 == 0 || offset > max_offset {
                break; // reached the top (or bounded walk end)
            }
            offset += 3;
        }
        let total = t2.elapsed();
        println!(
            "scroll-cold: {} wheel steps to the top, total {:>8.1} ms, mean {:>6.2} ms/step, worst {:>6.2} ms @ offset {}",
            steps,
            total.as_secs_f64() * 1e3,
            total.as_secs_f64() * 1e3 / steps as f64,
            worst.as_secs_f64() * 1e3,
            worst_at
        );
        println!(
            "             cache after walk: {} blocks, {} rows",
            cache.cached_blocks(),
            cache.cached_rows()
        );

        // Warm pass: everything visited is cached; walk back down.
        let mut offset2 = 0usize;
        let t3 = Instant::now();
        let mut steps2 = 0usize;
        loop {
            let (b0, _b1) = bench::walk_window(&mut cache, &entries, offset2, VIEWPORT, WIDTH);
            steps2 += 1;
            if b0 == 0 || offset2 > max_offset {
                break;
            }
            offset2 += 3;
        }
        let warm = t3.elapsed();
        println!(
            "scroll-warm: {} steps down, total {:>8.1} ms, mean {:>6.2} ms/step",
            steps2,
            warm.as_secs_f64() * 1e3,
            warm.as_secs_f64() * 1e3 / steps2 as f64
        );
    }

    // ---- typing: per-keystroke frame cost at the bottom of a huge transcript ----
    if only.is_none() || only == Some("typing") {
        let entries = load_entries();
        let mut cache = bench::BlockCache::new_public();
        let n = bench::block_count(&entries);
        let _ = bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);

        // A keystroke re-renders the bottom window (cache hits). The input
        // line itself is one Paragraph row — negligible vs history.
        let frames = 200;
        let t4 = Instant::now();
        for k in 0..frames {
            let buf = format!("打字测试 {}", k % 10);
            let _ = buf.len();
            let (rows, _, _) = bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);
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
        let _ = bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);

        // Locate ~a run with many tool results in the middle of the transcript.
        let mut found = None;
        for start in 0..entries.len().saturating_sub(120) {
            let window = &entries[start..start + 120];
            if window
                .iter()
                .filter(|e| matches!(e, Entry::ToolResult { .. }))
                .count()
                > 16
            {
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
        let (rows, _, _) = bench::window_at(&mut cache, &entries, bidx, (bidx + 30).min(n), WIDTH);
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
        // Walk the whole transcript once (worst-case cache fill).
        let mut offset = 0usize;
        loop {
            let (b0, _) = bench::walk_window(&mut cache, &entries, offset, VIEWPORT, WIDTH);
            if b0 == 0 {
                break;
            }
            offset += 60;
        }
        let (_, cached_rows, cached_blocks) =
            bench::window_bottom(&mut cache, &entries, 0, VIEWPORT, WIDTH);
        println!(
            "mem:         {} entries, {} blocks, cache {} blocks / {} rows (budget 256 blocks)",
            entries.len(),
            n,
            cached_blocks,
            cached_rows
        );
    }
}
