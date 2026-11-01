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
| 待办回填 | `Loop::todo(session)` |

- `Message` 与 `Change` 由 loop 导出；api 解自己 payload 后编成它们，不认识 agent 域的其他类型。
- 子会话的行由 loop 在 `Effect::Child` 落地时写，api 不参与。
- busy 看 api 自己的订阅表，不问 loop。

## ui 与 main

`main.rs` 和 `ui` 仍从 `ipc::contract` 取 `Viewer` 与 `ModelId`，两个类型都已不在 ipc——`contract`
现在只剩 `message` 与 `error`。它们要么搬进 api 的公开面，要么由 ui 自持。

## config

`call.config`、`call.pools`、`post.default_model`、`post.reload` 仍答 `internal`：config 的值由装配方
传给 api 与 loop，两个域自己都不读配置文件。