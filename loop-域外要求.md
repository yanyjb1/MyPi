# loop 域外要求

只写 loop 之外必须提供的形状。loop 本轮不改这些域的代码；调用按这里写。

## request

- 历史：从 loop 注入的读口读，不 `import storage`，不持 `Writer`。
- 解码：流式解码；增量与块边界直接写 loop 给的 channel。正文、思考各带来源说明；工具参数不带，下游靠 `call_id` 认。
- 写块：块级写库经 loop 给的写块口；块类型由 request 唯一决定；工具结果整批切；缺一个调用结果就整体失败。
- 交回：`Stop`、`Usage`、`call_id / name / args / intent`；压缩时交回摘要文本。
- 不派工具、不开回合、不碰会话状态、不认识工具名。
- artifact 引用：全文 FNV-1a 64，取 4/6/8/10/12/14/16 位十六进制前缀；同内容同引用；不同内容前缀冲突时加长。引用随块交给 loop。
- 压缩：读回合视图、求计划、发一次无工具请求、交回摘要和保留区区间；区间按原回合分组，切点落在回合内时取 `[z, y]`。不建第二套请求栈。

## tools

- 一切外部条件从 `Env` 拿；不开数据库、不读配置文件、不碰环境变量、不持 `Writer`。
- 结果：文本、memo、溢出全文、agent 自己的中性 effect、是否终止回合、进度增量。
- 不自己量输出；输出上限由前门量。
- `cd` 只改自己看到的目录；批内看到批开始的快照。
- 子代理只回意图：模板、变量、工具子集、预算、`provider:model`、cwd；不建会话行。未指定 `p:m` 时继承主代理。
- `final` 只给子会话；`task` 只给主会话；`kairos` 不给子会话。

## config

- 提供 provider 出口：`base_url`、`api`、`auth_header`、key 解析。
- 新增 `config.server.agent.loop.scrooge`；真 = 不许中途换模型。
- 删除 `config.server.agent.reloadOnCompaction`。
- 提供 browser 的 `bin` 与 `profileDir` 两个字段。
- agent 自读 `mcp.json`；配置里不再提供 mcp 段。

## api / 调用方

- 主会话：先写 `sessions` 行（`provider`、`model`、`meta`、`anchor = NULL`），拿 `sessionid`，再 `Loop::book`，再 `deliver`。
- 子会话：先写工具结果块，再写 `sessions` 行（`anchor = 块 id`），拿 `sessionid`，再 `book`，再 `deliver` 启动消息。
- `Birth`：`session`、`provider`、`model`、`system`、`tools`、`meta`、`cwd`。系统提示词和 tools 由调用方从 `meta` 或外部 profile 取好，loop 不读 profile。
- 投递：`deliver(session, Message)`；忙时 loop 自己排队。打断用 `interrupt(session)`。丢弃用 `drop(id)`。
- 状态变化：`Change::Model { provider, model }`、`Change::Cwd(path)`；profile 没有切换入口。
- 活线：`Loop::new` 时传 `Sender<Event>`；事件自带会话号，api 按订阅表分发。
- busy 判断看订阅表，不问 loop。
- 子会话列表从 `sessions.anchor` 读；不再有 `subsessions` 表。

## job / wake

- job 回调持有 `Mailbox`，`push_wake(text)`；wake 落库时块类型是 `wake`，发往上游时伪装成 user role。
- 用户插话走 `Mailbox::push_user`；可撤销、可修改。
- 两个队列都空、或用户打断，才结束回合。
- 阻塞点先收工具结果，再取 wake，再取用户消息。
