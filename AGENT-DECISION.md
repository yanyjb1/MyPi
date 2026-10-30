# AGENT-DECISION

本文件只保留仍然有效、且会影响下一步改造的决策。已完成的 loop 与 storage 不再重复：
现状与设计见 `src/server/agent/loop/README.md`、`src/server/agent/loop/ARCHITECTURE.md`、
`src/server/storage/README.md`、`src/server/storage/ARCHITECTURE.md`。
request、tools、config、api 的接口形状见 `loop-域外要求.md`。

## 1. 三方职责

```text
storage ──► loop ──► { request, tools }
```

- **loop**：会话、回合、信箱、行写入。已完成。
- **request**：解码上游流、切块、转发增量与边界、写块；不派工具、不开回合、不碰会话状态、不认识工具名。
- **tools**：纯函数表，一切外部条件经 `Env`；不开数据库、不读配置、不碰环境变量、不持 `Writer`。
- 依赖单向：loop → request、loop → tools；request 与 tools 互不相识。

## 2. request

- 历史通过 loop 注入的读口读，不 `import storage`。
- 解码后直接往 loop 给的 channel 写增量与块边界；正文和思考带来源说明，工具参数不带。
- 块级写库经 loop 给的写块口；块类型由 request 唯一决定；工具结果整批切，缺一个调用整体失败。
- 压缩读回合视图、求计划、发一次无工具请求，交回摘要与保留区区间。

## 3. tools

- 结果只交文本、memo、溢出全文、中性 effect、是否终止回合、进度增量。
- 不自己量输出；`cd` 只改自己看到的目录，批内看批开始快照。
- 子代理只回意图：模板、变量、工具子集、预算、`provider:model`、cwd、启动消息；未指定 `p:m` 继承主代理。
- `final` 只给子会话，`task` 只给主会话，`kairos` 不给子会话。

## 4. config / api

- config 提供 provider 出口、`config.server.agent.loop.scrooge`、browser 两个字段；删除
  `config.server.agent.reloadOnCompaction`；agent 自读 `mcp.json`。
- api 持有 `Loop` 句柄；主会话先写 `sessions` 行再 `book`，子会话由 loop 写行并 `book`。
- 实时输出走 `Sender<Event>`；busy 看订阅表，不问 loop。
- 子会话列表从 `sessions.anchor` 读；不再有 `subsessions` 表。
