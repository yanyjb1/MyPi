use mypi::server::agent::loop_rs::ToolExecutor;
use mypi::server::agent::tools::BuiltinTools;
fn main() -> anyhow::Result<()> {
    let mut tools = BuiltinTools::new(std::env::current_dir()?);
    let call = mypi::server::ai::types::ToolCall {
        id: "f1".into(),
        kind: "function".into(),
        function: mypi::server::ai::types::FunctionCall {
            name: "fetch".into(),
            arguments:
                r#"{"intent":"读文档","url":"doc.rust-lang.org/book/ch17-00-async-await.html"}"#
                    .into(),
        },
    };
    let out = tools.execute(&call, &mut |_| {})?.text;
    println!("{}", &out[..out.len().min(400)]);
    Ok(())
}
