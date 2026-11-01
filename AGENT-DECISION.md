# AGENT-DECISION

本文件只保留仍然有效、且会影响下一步改造的决策。已完成的 loop、storage 与 tools 不再重复：
现状与设计见 `src/server/agent/loop/README.md`、`src/server/agent/loop/ARCHITECTURE.md`、
`src/server/storage/README.md`、`src/server/storage/ARCHITECTURE.md`、
`src/server/agent/tools/README.md`、`src/server/agent/tools/ARCHITECTURE.md`。
request 的接口形状见该域 README；api、ui 与 config 的剩余接线见 `api-loop-目标形态.md`。

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
- 溢出全文只交引用与全文，二者随块交给 loop 落库；request 不写任何文件。
- 压缩读回合视图、求计划、发一次无工具请求，交回摘要与保留区区间。

## 3. tools

- 结果只交文本、memo、中性 effect、是否终止回合、进度增量；溢出由前门量、由 loop 落库。
- 不自己量输出；`cd` 只改自己看到的目录，批内看批开始快照。
- 子代理只回意图：模板、变量、工具子集、预算、cwd、启动消息、答案名；system 与模型继承本会话；
  会话行由 loop 在 effect 落地时写。
- 工具是否可用只由名单决定：主会话看 profile 的工具名单，子会话看模板的名单。
  `task` 在派发时从子会话名单里静默剔除；`final` 自动补进子会话名单，主代理不写也不知道。
  工具层不设第二道门，`kairos`、`mcp`、`web` 一律看名单。
- 摘要按工具各一份，挂在工具声明旁；没有摘要的工具在 `context` 里退回它的参数文本。
- `todo` 只有三条操作：`set`（整表替换，空表即清空）、`tick`（按序号打勾，`done:false` 取消）、
  `view`（只读）。扁平表、布尔状态、无阶段、无开关、无 blocker；序号越界整批拒绝。
- `todo` 不存状态：`set` 的结果 memo 带 `"todo_reset":true`，最新的那条是重放锚点，用它的 memo
  作种子，再折叠其后的调用。

## 4. config / api

- config 提供 provider 出口、`config.server.agent.loop.scrooge`；删除
  `config.server.agent.reloadOnCompaction`；agent 自读 `mcp.json`；browser 字段由 web 域自读，
  不经工具层传递。
- loop 从 config 取值构造工具层：`Loop::new(writer, settings, ask, out, &cfg)`。
- api 持有 `Loop` 句柄；主会话先写 `sessions` 行再 `book`，子会话由 loop 写行并 `book`。
- `Birth.tools` 是名单；system 与名单分开解析。
- 实时输出走 `Sender<Event>`；busy 看订阅表，不问 loop。
- 子会话列表从 `sessions.anchor` 读；不再有 `subsessions` 表。
