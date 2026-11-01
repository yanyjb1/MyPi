# agent 剩余接线

request、loop、tools 的封装层、config、storage 的读口都已落地。形状看各自的 `README.md` 与
`ARCHITECTURE.md`，工具层目标看 `tools-目标形态.md`。这里只留下一步要接的东西。

## 一、工具体

共性：全部改吃 `tools::contract::Env`，不引 storage、不引 config；`ToolOutput` 里没有溢出，
溢出由前门做；effect 由工具自己构造，`ts` 留空；`Tool.summary` 各补一份，`context` 会调它。

逐件：

- `read` / `write` / `edit`：只用 `Env.workspace`。
- `cd`：`Env.workspace.chdir` 写回共享把手，下一批生效。
- `alias`：批内的 `Env.aliases` 副本，产出 `Alias` effect；guard 照旧。
- `bash`：`Env.aliases`；`#ref` 改成 `Env.export.path(reference)` 换到的路径；guard 与 shell 照旧。
- `context`：`op = query | show`；`query` 走 `Source::history`，`show` 走 `Source::block`；
  渲染超过十行落 `Env.export.dir()`，`show` 超过三个块号也落；旧 `Window` 一并删。
- `memo`：读走 `Source::memo`，写回 `Memo` effect；current 只写，global 只读。
- `todo`：先看 `Source::todo_on`，再用 `Source::todo_off` 与 `Source::calls_after(..., "todo")`
  重放；纯函数放 tools；转场时往 memo 写 `todo_on: true|false`。
- `kairos`：`Env.source.jobs(session)`；命令 runner、别名、导出口照旧。
- `task`：模板与变量、子集检查，只产 `Child { tools, cwd, start, max_tool_calls }`；
  system 与模型继承本会话，不进 effect；模板在 `Env.paths.agents`；默认额度 `Env.default_budget`。
- `final`：`Env.source.child(session)` 的 `Draft`；`submit` 写 `Env.paths.answers/<子会话号>.md`，
  且必须是本批唯一调用。
- `mcp`：桥从 `Env.mcp` 来，工具只列与调。
- `web`：改吃 `Env.browser`；行为不变。

顺手删 `tools/memo/memo/read.rs`：读口已经搬到 `loop/session/source.rs`。

## 二、job

- `JobRunner`、`JobCtx`、`JobOutcome`、`JobSpec`、`WakeSink` 抬成公开面，loop 才实现得了。
- 工厂与 `JobService` 由 loop 按会话构造：并发数、超时、输出上限、保留数由 loop 读 config 传值；
  唤醒口接会话的 `Mailbox`。
- 子代理的 runner 归 loop：跑子会话、收尾草稿；kairos 的 runner 留 tools。
- reserve/activate 归 loop，`task` 不碰 `JobService`。
- 构造好之后经 `Db::attach_jobs` 挂上。

## 三、api 与启动

- 启动：`Config` 取值后 `Loop::new(writer, settings, ask, out, &cfg)`；直播线一条
  `Sender<request::Event>`，request 与 loop 共用。
- `Birth.tools` 传名单；system 与名单分开解析。切 profile 下一回合自然生效，不重开书。
- `call todo`：api 向 loop 要一次重放结果（loop 内调 tools 的纯函数），把清单交给前端作为恢复
  时的初始量，之后前端按后续结果的 memo 增量维护。
- `close_web_session` 照旧。

## 四、已知缺口

- 同一批里尚未落库的调用，`Source` 看不见，`current` 标签不可达；要么给批输入带当前批的记录，
  要么去掉这个标签。
- storage 还有三处编译错：写口调 `apply` 的位置、`reader` 里 `request_json` 的参数个数、
  `Row::Memo` 的 anchor 类型。属于 storage 自己的债。
