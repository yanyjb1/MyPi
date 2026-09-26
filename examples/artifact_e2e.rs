//! End-to-end artifact spill test: a 50k-line `tree` output through the
//! real bash tool + #N reference round trip. Not a unit test (touches the
//! real DB + shell + temp dirs); run as an example.
//!
//! Usage: cargo run --release --example artifact_e2e

use mypi::server::agent::artifacts::ArtifactStore;
use mypi::server::agent::loop_rs::ToolExecutor as _;
use mypi::server::agent::tools::BuiltinTools;
use std::sync::{Arc, Mutex};

fn main() {
    mypi::tui::theme::init(None);

    // Real DB in /tmp, session 1.
    let db = std::env::temp_dir().join("mypi-artifact-e2e.db");
    let _ = std::fs::remove_file(&db);
    let mut store = mypi::server::store::Store::open(&db).expect("db");
    store.create_session("e2e", "/tmp").expect("session");
    let arc = Arc::new(Mutex::new(store));
    let art = ArtifactStore::new(arc.clone(), 1);

    let mut tools = BuiltinTools::new(std::env::temp_dir()).with_artifacts(art.clone());

    // ---- step 1: a tree that yields 50k+ lines ----
    let gen_dir = std::env::temp_dir().join("mypi-art-e2e-fixture");
    let _ = std::fs::remove_dir_all(&gen_dir);
    std::fs::create_dir_all(&gen_dir).unwrap();
    // 40 dirs x 1250 files = 50k+ lines from tree.
    for d in 0..40 {
        let sub = gen_dir.join(format!("branch_{d}"));
        std::fs::create_dir_all(&sub).unwrap();
        for f in 0..1250 {
            std::fs::write(sub.join(format!("leaf_{d}_{f}.txt")), "x").unwrap();
        }
    }
    println!("== step 1: tree (50k+ lines) through the bash tool ==");
    let t0 = std::time::Instant::now();
    let out = tools
        .execute(
            &serde_json::from_value(serde_json::json!({
                "id": "call_e2e_1",
                "type": "function",
                "function": {
                    "name": "bash",
                    "arguments": format!("{{\"command\":\"tree {}\"}}", gen_dir.display())
                }
            }))
            .unwrap(),
            &mut |_| {},
        )
        .expect("tree run")
        .text;
    let dt = t0.elapsed();
    println!(
        "  tool returned {} bytes in {:.1} ms",
        out.len(),
        dt.as_secs_f64() * 1e3
    );
    println!("  placeholder head: {}", out.lines().next().unwrap_or(""));
    assert!(out.contains("已存为巨物 #"), "输出必须是巨物占位符");
    let id: i64 = out
        .split('#')
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse().ok())
        .expect("占位符里的 id");
    println!("  artifact id = {id}");

    // ---- step 2: the model pulls a range back through #id ----
    println!("== step 2: #id | grep | head through the bash tool ==");
    let t1 = std::time::Instant::now();
    let out2 = tools
        .execute(
            &serde_json::from_value(serde_json::json!({
                "id": "call_e2e_2",
                "type": "function",
                "function": {
                    "name": "bash",
                    "arguments": format!("{{\"command\":\"#{id} | grep branch_7 | head -5\"}}")
                }
            }))
            .unwrap(),
            &mut |_| {},
        )
        .expect("reference run");
    let dt2 = t1.elapsed();
    println!(
        "  {} bytes in {:.1} ms",
        out2.text.len(),
        dt2.as_secs_f64() * 1e3
    );
    for l in out2.text.lines().take(6) {
        println!("  > {l}");
    }
    assert!(out2.text.contains("branch_7"), "引用结果必须包含 grep 命中");

    // ---- step 3: second-order artifact (no filter = spill again) ----
    println!("== step 3: cat #id (no filter) — must spill again ==");
    let out3 = tools
        .execute(
            &serde_json::from_value(serde_json::json!({
                "id": "call_e2e_3",
                "type": "function",
                "function": {
                    "name": "bash",
                    "arguments": format!("{{\"command\":\"cat #{id}\"}}")
                }
            }))
            .unwrap(),
            &mut |_| {},
        )
        .expect("cat run");
    assert!(out3.text.contains("已存为巨物 #"), "无过滤取用必须再次巨物化");
    let id2: i64 = out3.text
        .split('#')
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse().ok())
        .unwrap();
    println!("  second artifact id = {id2}");
    assert!(id2 > id, "递归巨物必须拿到新 id");

    // ---- step 4: session delete cascades ----
    println!("== step 4: delete_session cascades artifacts ==");
    arc.lock().unwrap().delete_session(1).expect("delete");
    assert!(art.fetch(id).unwrap().is_none(), "删除会话必须带走巨物");
    println!("  artifacts gone with the session");

    // Cleanup.
    let _ = std::fs::remove_dir_all(&gen_dir);
    let _ = std::fs::remove_file(&db);
    println!("ALL OK");
}
