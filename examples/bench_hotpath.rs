//! Micro-benchmark: isolate the per-frame hot-path costs the panorama
//! flagged, so optimization targets evidence, not hunch.
//!
//!   1. `blocks::blocks(entries)`          — the O(n) grouping scan
//!   2. full frame (sync + window + rows)  — what view.rs does per paint
//!   3. JSON serialization of the context  — per LLM call, not per frame
//!
//! Usage: MYPI_BENCH_DB=/tmp/mypi-bench/scale-24000.db \
//!        cargo run --release --example bench_hotpath

use std::time::Instant;

fn db_path() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("MYPI_BENCH_DB").unwrap_or_else(|_| "/tmp/mypi-bench/scale-24000.db".into()),
    )
}

fn load_entries() -> Vec<mypi::entry::Entry> {
    let store = mypi::store::Store::open(&db_path()).expect("open bench db");
    // The bench DB has one session (id 1) with a linear chain.
    store.load_entries(1).expect("load entries")
}

fn main() {
    mypi::tui::theme::init(None);
    let entries = load_entries();
    let n = entries.len();
    println!("entries: {n}");

    // ---- 1. blocks::blocks() alone ----
    const ITERS: usize = 200;
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..ITERS {
        acc += mypi::tui::transcript::blocks::blocks(&entries).len();
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / ITERS as f64;
    println!("blocks::blocks()   : {per:>8.4} ms/call  ({acc} blocks total)");

    // ---- 2. full frame (the real per-paint path) ----
    let mut cache = mypi::tui::bench::BlockCache::new_public();
    let width = 100usize;
    let viewport = 40usize;
    // warm
    let _ = mypi::tui::bench::window_bottom(&mut cache, &entries, 0, viewport, width);
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = mypi::tui::bench::window_bottom(&mut cache, &entries, 0, viewport, width);
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / ITERS as f64;
    println!("full frame (bottom): {per:>8.4} ms/frame");

    // ---- 3. context serialization (per LLM call, not per frame) ----
    // Reconstruct a protocol context from the entries, then serialize it
    // the way client.rs does: to_value() (builds a Value tree) + to_string
    // (walks it again).
    let ctx = mypi::server::turn::entries_to_context("sys", &entries);
    let t = Instant::now();
    for _ in 0..ITERS {
        let v = serde_json::to_value(&ctx.messages).unwrap();
        let _ = serde_json::to_string(&v).unwrap();
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / ITERS as f64;
    println!(
        "ctx serialize (2x)  : {per:>8.4} ms/call  ({} messages)",
        ctx.messages.len()
    );

    // 3b. single-pass serialize (what it *could* be)
    let t = Instant::now();
    for _ in 0..ITERS {
        let _ = serde_json::to_string(&ctx.messages).unwrap();
    }
    let per = t.elapsed().as_secs_f64() * 1e3 / ITERS as f64;
    println!("ctx serialize (1x)  : {per:>8.4} ms/call  (single pass baseline)");
}
