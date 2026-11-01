# api-loop 剩余接线

ipc 的管道和 api 的门都已经落地。剩下的是把 api 接到 loop 上，不是设计问题。

## api → loop

api 的门还绑着 `agent::r#loop::register::{Message, Retention}` 与 `agent::r#loop::outlet::Event`，
这两个模块不存在。loop 的对等物是 `Loop`：

| api 要的 | loop 现在给的 |
|---|---|
| 挂号 | `Loop::book(Birth) -> Result<i64>` |
| 投消息 | `Loop::deliver(session, Message) -> Result<Option<i64>>`；忙时 loop 自己排队 |
| 打断 | `Loop::interrupt(session)` |
| 收掉 | `Loop::drop(session)` |
| 换模型、换目录 | `Loop::change(session, Change)` |
| 活线 | `Loop::new(writer, settings, ask, out, &cfg)` 里的 `Sender<Event>` |
| 读信箱 | `Loop::mailbox(session)` |
| 待办回填 | `Loop::todo(session) -> Result<Vec<TodoItem>>` |

- `Message` 与 `Change` 由 loop 导出；api 解自己 payload 后编成它们，不认识 agent 域的其他类型。
- 子会话的行由 loop 在 `Effect::Child` 落地时写，api 不参与。
- busy 看 api 自己的订阅表，不问 loop。

## api → tools

工具层的根不再转发任何名字。api 现在写下的 `tools::Env`、`tools::TodoPhase`、`tools::TodoTask`、
`tools::TodoStatus`、`tools::execute` 全部作废，`TodoPhase`/`TodoStatus`/`TodoTask`/`execute`
在工具层从来不存在。

| api 要的 | 现在的路径 |
|---|---|
| `Env` | `tools::contract::env::Env` |
| `ToolResult` | `tools::contract::output::ToolResult` |
| `Effect` / `MemoOp` / `AliasOp` | `tools::contract::output::{Effect, MemoOp, AliasOp}` |
| `Batch` / `BatchInput` / `Cwd` | `tools::contract::call::{Batch, BatchInput, Cwd}` |
| `Source` / `Child` / `Export` / `Draft` | `tools::contract::view::{...}` |
| `TodoItem` | `tools::memo::todo::state::TodoItem` |
| 工具表与手册 | `tools::registry::{TOOLS, roster}` |
| 关掉一个会话的浏览器标签 | `tools::web::close_session(session)` |

关于 `tools::web::close_session`：api 不能直接引用 `crate::web`（`src/lib.rs` 的分层规则禁止），
所以这一层转发是必需的，不是遗留。它只在 `web` feature 打开时存在。

关于 todo：`Loop::todo` 交出的是**已经重放好的整表**，元素是 `TodoItem`。没有阶段，没有
`blocked`/`abandoned` 这些状态，只有一项是否打勾。`convert.rs` 里那套五状态映射要删掉，
换成一个布尔和一个可选的行号。

关于 mcp：工具层不再有"本地配了服务器才放行"这道门。会话说 `mcp` 就有 `mcp`，没有服务器时
调用会如实报未配置。api 不需要判断。

关于 browser：启动参数由 `src/web` 自己从 config 读，不经过 `Env`，也不经过 api。api 不做浏览器
相关的任何事。

## ui 与 main

`main.rs` 和 `ui` 仍从 `ipc::contract` 取 `Viewer` 与 `ModelId`，两个类型都已不在 ipc——`contract`
现在只剩 `message` 与 `error`。它们要么搬进 api 的公开面，要么由 ui 自持。

## config

`call.config`、`call.pools`、`post.default_model`、`post.reload` 仍答 `internal`：config 的值由装配方
传给 api 与 loop，两个域自己都不读配置文件。

浏览器两项（`browser.bin`、`browser.profileDir`）是例外：`src/web` 自己读，装配方不传。
