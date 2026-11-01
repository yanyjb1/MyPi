# agent 剩余接线

tools 与 loop 两域的异步这一族（`jobs`、`kairos`、`task`、`final`）已经接完，形状见
`src/server/agent/tools/README.md` 与 `src/server/agent/loop/README.md`。这里只剩两件。

## mcp

`tools/mcp` 还绑着已删的 `spec::Env`，`sync.rs` 与 `stdio.rs` 还引着已删的
`config::McpConfig`、`config::McpServerConfig`。

- 格式与定位：`design/agent/tools/mcp.md`。
- 装配点：`Loop::new`；挂载点：`loop/session/source.rs` 的 `attach_mcp`。
- 现状：`Furnish::mcp` 答 `None`，`mcp` 不进 roster。

## web

`tools/web/mod.rs` 还绑着旧的 `spec::Env` 与 `env.inbox`。`web` 是默认 feature，于是默认构建编不过。

api、ui 与 config 的接线见 `api-loop-目标形态.md`。