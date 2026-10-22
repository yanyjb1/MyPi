# MCP 设计

日期：2026-10-22

状态：核心设计收口，进入施工。

---

## 1. 定位

MCP 只做一个静态工具 `mcp`，不参与动态工具注册表，不塞进 bash，不改 `executor`。

实际执行在外部 MCP server 进程里；MyPi 是 host/client，只负责发现工具、转发调用、把结果交回模型。

第一版范围：

- 只做 stdio。
- 只做 tools。
- 只做同步调用。
- 不做 resources、prompts、HTTP、OAuth、Tasks。

---

## 2. 工具面

```text
mcp {
  op: list | call,
  server?: string,
  tool?: string,
  args?: object
}
```

### list

列出 server 和工具：

```text
server: local
  md_refs — List reference files and resources...
  md_search — Search resources by keyword...
```

只给名字、描述、必填参数名，不吐完整 JSON Schema。模型对 JSON 更熟，完整 schema 既费 token 又很少真被读。

### call

```text
mcp {
  op: "call",
  server: "local",
  tool: "md_search",
  args: { "query": "民法典", "scope": "title" }
}
```

`args` 直接是 JSON 对象，不走 shell，所以没有引号和转义问题。省略就是 `{}`。

参数错了让 MCP server 自己报错，模型再改。MyPi 不替它做 schema 校验。

---

## 3. 接入方式

`TOOLS` 里加一个静态 `mcp::TOOL`。`Env` 里加：

```rust
pub mcp: Option<Arc<dyn McpBridge>>,
```

`roster` 只在配置了 server 时把 `mcp` 放进去。`executor` 完全不知道 MCP，照常 `Args::fill`，`mcp` 工具内部拿 `args` 的 JSON 去调 bridge。

不做动态工具注册，不做 `Route` 表，不做 `mcp__server__tool` 命名空间。MCP 只有一个入口：`mcp`。

---

## 4. McpBridge

桥是同步 trait，不出现 `rmcp` 类型：

```rust
pub trait McpBridge: Send + Sync {
    fn servers(&self) -> Vec<ServerInfo>;
    fn tools(&self, server: &str) -> Result<Vec<ToolInfo>>;
    fn call(&self, server: &str, tool: &str, args: Value) -> Result<ToolOutput>;
    fn shutdown(&self);
}
```

`ServerInfo`、`ToolInfo`、`ToolOutput` 都是 MyPi 自己的类型。工具层只认这个 trait，底下换实现不影响上层。

---

## 5. 自带同步实现

第一版自带 `SyncMcpBridge`，只用现有依赖：

- `std::process` 起子进程。
- stdin/stdout/stderr 都接管道。
- stdout 一行一个 JSON-RPC；reader 线程解析后按 `id` 分发给等待中的调用。
- stderr 单独排空，只当日志，不能堵住管道。
- 主线程发请求，把 `id` 和等待通道放进 map，阻塞等回复，带超时。
- 子进程退出时，所有等待中的调用全部报错。
- `Drop` 时杀子进程，避免僵尸。

握手先走旧版 `initialize`，收到结果后发 `notifications/initialized`。然后 `tools/list`、`tools/call`。MCP 2026-07-28 的 `server/discover` 生命周期第一版不做；遇到只认它的 server，走 rmcp feature。

每个 server 一个子进程，懒启动。同一个 MyPi 进程内，多个会话共享同一个 bridge。

---

## 6. 配置

```yaml
mcp:
  servers:
    local:
      command: "uv"
      args: ["run", "--project", "/path/to/MCP", "python", "mcp_service.py"]
      env: {}
      cwd: "/path/to/MCP"
      timeoutSecs: 120
```

- `command`、`args`、`env`、`cwd` 都必填或有明确默认。
- stdout 只走协议；日志必须去 stderr。
- 每个调用必须有超时，不能无限等。
- 第一版不做自动重启；进程死了就报错，下一次调用可以重新拉起。

---

## 7. 结果映射

MCP 的 `content` 是数组：

- 文本块按顺序拼接。
- `isError` 映射成 `ok = false`。
- 图片、音频、结构化 content 第一版不支持，遇到就报“unsupported content”或忽略并注明。
- 结果仍然走 `ToolOutput`，过长由 front door 走 spill/artifact。

---

## 8. feature

```toml
[features]
mcp = []                 # 工具 + trait + 自带同步实现
mcp-rmcp = ["dep:rmcp", "dep:tokio"]  # 换成 rmcp 实现
```

用 `cfg` 选一个实现。两个同时开就 `compile_error!`。

`mcp-rmcp` 的实现：

- 单独线程跑一个 tokio runtime。
- 用 `rmcp` 连 server。
- 对外仍然实现同步 `McpBridge`：请求丢进命令通道，tokio 线程执行 async 调用，oneshot 把结果送回；调用方阻塞等，带超时。
- `shutdown` 停 runtime、关 client、收子进程。
- tokio 只活在这个线程里，不渗进主循环。

上层 `mcp` 工具、配置、结果格式化完全不变，只换 `Arc<dyn McpBridge>` 背后是谁。

---

## 9. 安全

MCP server 是用户自己接入的，信任审查是用户的责任。

第一版不做沙箱、不做自动审批、不做 server 白名单。`mcp` 工具本身不执行 shell；它只把 JSON 转发给用户配置的 server。用户放行一个 server，就等于放行它暴露的工具。

---

## 10. 明确不做

- 不把 MCP 塞进 bash。
- 不做动态工具注册表。
- 不做 `mcp__server__tool` 命名空间。
- 不做 `mcp://` URI。
- 不做 resources、prompts。
- 不做 HTTP、SSE、OAuth。
- 不做 MCP Tasks 扩展。
- 不把完整 JSON Schema 塞给模型。
- 不让 tokio 进入主循环。
