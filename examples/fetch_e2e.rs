use mypi::web::fetch::{FetchArgs, fetch};
fn main() -> anyhow::Result<()> {
    let args = FetchArgs {
        intent: "e2e".into(),
        url: std::env::args().nth(1).expect("url"),
        raw: None,
    };
    let t = std::time::Instant::now();
    let out = fetch(&args)?;
    eprintln!(
        "[elapsed {:.1}s, {} chars]",
        t.elapsed().as_secs_f32(),
        out.chars().count()
    );
    println!("{}", &out[..out.len().min(1200)]);
    Ok(())
}
