// 端到端：search() 直连 tier → 兜底 tier（真实 Bing + Helium CDP）
use mypi::web::{SearchArgs, search, render};

fn main() -> anyhow::Result<()> {
    let args = SearchArgs {
        intent: "验证 search 全链路".into(),
        query: std::env::args().nth(1).unwrap_or_else(|| "rust async book".into()),
        limit: Some(5),
    };
    let t0 = std::time::Instant::now();
    let hits = search(&args)?;
    eprintln!("[elapsed {:.1}s, {} hits]", t0.elapsed().as_secs_f32(), hits.len());
    print!("{}", render(&hits));
    Ok(())
}
