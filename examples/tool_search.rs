// 通过正式工具层跑一次 search（验证注册/分发/渲染全链）
use mypi::server::agent::loop_rs::ToolExecutor;
use mypi::server::agent::tools::BuiltinTools;
fn main() -> anyhow::Result<()> {
    let mut tools = BuiltinTools::new(std::env::current_dir()?);
    let call = mypi::server::ai::types::ToolCall {
        id: "t1".into(),
        kind: "function".into(),
        function: mypi::server::ai::types::FunctionCall {
            name: "search".into(),
            arguments:
                r#"{"intent":"验证工具注册","query":"site:github.com rust async","limit":3}"#.into(),
        },
    };
    let out = tools.execute(&call, &mut |_| {})?.text;
    println!("{out}");
    Ok(())
}
