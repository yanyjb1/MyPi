# agent 剩余接线

tools 的封装层与 bash 一族（`bash`、`read`、`write`、`edit`、`cd`、`alias`、`context`）、`memo`、
`todo` 都已落到新契约。形状看 `src/server/agent/tools/README.md` 与 `ARCHITECTURE.md`；这里只列
还没接的东西。

## 一、工具体还剩五件

共性：改吃 `tools::contract::Env`，不引 storage、不引 config；`ToolOutput` 里没有溢出；effect 由
工具自己构造、`ts` 留空；`Tool.summary` 各补一份。

- `kairos`：`Env.source.jobs(session)`。命令 runner 已随 bash 改好（alias、guard、`#ref`、壳），
  `kairos/mod.rs` 与 `service.rs` 仍是旧 `Env`。
- `task`：模板与变量、子集检查，只产 `Child { tools, cwd, start, max_tool_calls }`；system 与模型
  继承本会话。
- `final`：`Env.source.child(session)` 的 `Draft`；`submit` 写 `Env.paths.answers/<子会话号>.md`，
  且必须是本批唯一调用。
- `mcp`：桥从 `Env.mcp` 来。`mcp/stdio.rs` 与 `sync.rs` 现在直接引 `config::McpConfig`，要改成
  值由装配方传进来。
- `web`：`env.session` 一处字段；浏览器默认路径与默认 UA 是平台值。

## 二、job

- `JobRunner`、`JobCtx`、`JobOutcome`、`JobSpec`、`WakeSink` 抬成公开面，loop 才实现得了。
- 工厂与 `JobService` 由 loop 按会话构造：并发数、超时、输出上限、保留数由 loop 读 config 传值；
  唤醒口接会话的 `Mailbox`。
- 子代理的 runner 归 loop：跑子会话、收尾草稿；kairos 的 runner 留 tools。
- reserve/activate 归 loop，`task` 不碰 `JobService`。
- 构造好之后经 `Db::attach_jobs` 挂上。

## 三、subagent

- loop 写子会话行（`anchor` = 结果块）、登记任务、投递 `start`；`Child`（草稿 + 额度）活在会话
  活状态里。
- `subagent/service.rs`、`spawn.rs` 仍绑 `storage::Writer` 与 `EffectKind`，按新 effect 重写。

## 四、mcp 装配

装配方造 `SyncMcpBridge`，经 `Db::attach_mcp` 挂上；`Furnish::mcp` 现在答 `None`。

## 五、暂时不动的

- api 整层还在旧架构（`loop::register`、`loop::outlet`、`tools::execute`），`Loop::todo` 已给值，
  没人取。
- ui 与 api 仍引用已删掉的 `sessions.todo_on` 与两个 `todo_on`/`todo_off` post；storage 侧已删，
  这两处随各自波次收。
