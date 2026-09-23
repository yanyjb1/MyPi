use mypi::agent::loop_rs::ToolExecutor;
use mypi::agent::tools::BuiltinTools;
fn main() -> anyhow::Result<()> {
    let mut tools = BuiltinTools::new(std::env::current_dir()?);
    let call = mypi::ai::types::ToolCall {
        id: "f1".into(),
        kind: "function".into(),
        function: mypi::ai::types::FunctionCall {
            name: "fetch".into(),
            arguments: r#"{"intent":"读文档","url":"doc.rust-lang.org/book/ch17-00-async-await.html"}"#.into(),
        },
    };
    let out = tools.execute(&call)?;
    println!("{}", &out[..out.len().min(400)]);
    Ok(())
}
