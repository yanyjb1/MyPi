# AGENT-DECISION

本文件只保留仍然有效、且会影响下一步改造的决策。storage、loop、request、tools 四域已定型，
不再重复；现状与设计见各域的 `README.md` 与 `ARCHITECTURE.md`。api 的接线已落成，见
`src/server/api/README.md` 与 `src/server/api/ARCHITECTURE.md`；剩下的只有 ui 与 main 的接线。

## config / api

- config 提供 provider 出口、`config.server.agent.loop.scrooge`；删除
  `config.server.agent.reloadOnCompaction`；agent 自读 `mcp.json`；browser 字段由 web 域自读，
  不经工具层传递。
- loop 从 config 取值构造工具层：`Loop::new(writer, settings, ask, out, &cfg)`。
- api 持有 `Loop` 句柄；主会话先写 `sessions` 行再 `book`，子会话由 loop 写行并 `book`。
- `Birth.tools` 是名单；system 与名单分开解析。
- 实时输出走 `Sender<Event>`；busy 看订阅表，不问 loop。
- 子会话列表从 `sessions.anchor` 读；不再有 `subsessions` 表。
