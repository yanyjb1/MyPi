use mypi::agent::loop_rs::ToolExecutor;
use mypi::agent::tools::BuiltinTools;
use serde_json::json;

fn call(tools: &mut BuiltinTools, name: &str, args: serde_json::Value) -> anyhow::Result<String> {
    tools.execute(&mypi::ai::types::ToolCall {
        id: "e".into(),
        kind: "function".into(),
        function: mypi::ai::types::FunctionCall {
            name: name.into(),
            arguments: args.to_string(),
        },
    })
}

fn main() -> anyhow::Result<()> {
    let mut tools = BuiltinTools::new(std::env::current_dir()?);
    println!(
        "{}",
        call(
            &mut tools,
            "browser",
            json!({
                "intent":"open", "command":"open", "url":"doc.rust-lang.org/book/"
            })
        )?
    );
    println!(
        "{}",
        call(
            &mut tools,
            "browser",
            json!({
                "intent":"act eval", "command":"act", "op":"eval",
                "value":"document.title"
            })
        )?
    );
    let out = call(
        &mut tools,
        "browser",
        json!({
            "intent":"read", "command":"read"
        }),
    )?;
    println!(
        "[read] {} chars, has FOREWORD: {}",
        out.len(),
        out.contains("Foreword")
    );
    let shot = call(
        &mut tools,
        "browser",
        json!({
            "intent":"shot", "command":"screenshot", "path":"/tmp/tool_shot.png"
        }),
    )?;
    println!("{shot}");
    Ok(())
}
