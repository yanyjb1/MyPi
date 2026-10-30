# agent 剩余接线

request 与 loop 两块已建成，形状与现状各看自己的 `README.md`、`ARCHITECTURE.md`。这里只留还没接上的部分。

## 一、config

- 新增 `config.server.agent.loop.request.maxRetries`、`retryInterval`、`retryTimeout` 三个键，并写进自己的文档。

## 二、启动处

- 组装 request 的 `Options`：provider 表（名字、地址、协议号、密钥、是否带鉴权头）、重试三个数、输出上限（必填）、流式粒度、默认保留量。
- 组装 loop：`Loop::new(writer, settings, ask, batch, out)`；`settings` 现在只有 `{maxToolCalls, scrooge}`，输出上限不在这里。
- 直播线：一条 `Sender<request::Event>`，request 与 loop 共用。

## 三、api

- `Birth` 现在多一个 `compact` 字段：压缩指令模板（profile 的 `compact.md` 优先，其次 `instructionFile`）。`system`、`tools` 同理仍由调用方解析。
- 主会话先写 `sessions` 行再 `book`，子会话由 loop 写行；投递、打断、丢弃照旧。
- todo 用 `WireCall`；`message::ToolCall` 已不存在。

## 四、tools

- 实现 `Batch`：`tools::ToolOutput` 转成 loop 的 `ToolResult`（溢出用 `request::Spill`，不再自带 artifact）；入参的调用是 `request::ToolCall`（调用号、工具名、参数、意图）。
- context 工具跟随 `Message::Call`、`Message::Result` 两个变体。
- 取全文按引用找文件，不再按块号。

## 五、入口

- `main.rs`、`cli.rs` 补 `--http-worker`。不补则常驻子进程起不来，每次请求退化成直连，取消只能在分片之间生效。

## 六、storage 挡路的三处编译错

这三处不在本轮范围，但会让整棵树编不过，先记下（证据来自一次只读的类型检查，未改动它们）：

1. `writer/thread.rs`：`use super::super::write;` 之后调 `write::apply(...)`，而 `apply` 在 `write` 下是模块，不是函数。
2. `reader.rs:103`：`request_json(id, &hash, meta)` 传了三个参数，函数只收两个。
3. `write/apply.rs:122`：`Row::Memo { anchor }` 按 `Option<Ref>` 匹配，但 `shape::Row::Memo.anchor` 是 `Ref`。
