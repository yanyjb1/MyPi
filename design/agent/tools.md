# Tool 设计

日期：2026-10-22

状态：定稿，可施工。

---

## 1. 目录

工具按使用域分目录。域目录只做归类，不共享执行状态。

```text
tools/
  bash/
    bash/
    read/
    write/
    edit/
    alias/
    cd/
    context/
  web/
    mod.rs
  memo/
    memo/
    todo/
  kairos/
    mod.rs
  mcp/
    mod.rs
  subagent/
    task/
    final/
```

说明：

- `bash` 域里的工具都按 cwd 工作；`context` 只是归类，不读 cwd，也不执行 bash。
- `web` 只有一个工具，直接放 `mod.rs`。
- `memo` 与 `todo` 同域，但不共享表、不共享状态。
- `kairos`、`mcp` 各自一个工具。
- `subagent` 放 `task` 和 `final`。
- 共享的 `spec`、`registry`、`executor`、`workspace` 留在 `tools/` 根。

---

## 2. 工具清单

合并 `edit` 与 `mass_edit`，合并 `fetch`、`search`、`browser` 为 `web` 后：

现有工具：

```text
read
edit
write
cd
bash
todo
context
web
```

新增工具：

```text
memo
alias
kairos
task
final
mcp
```

最大并集 14 个。

- 主代理最大 13 个：没有 `final`。
- 子代理最大 12 个：没有 `task`，默认也没有 `web`。
- `mcp` 是条件工具，没配置 server 就不进 roster。
- profile 和子代理模板还可以继续裁剪。

---

## 3. 调用生命周期

每个工具调用是独立单元。调度器不按数组顺序执行，也不靠顺序表达依赖。

每次调用有自己的：

- 参数
- cwd 快照
- 输出通道
- 取消标记

工具之间不共享可变状态。

工具声明自己需要的资源：

- `read`、`context`：只读，可并行。
- `write`、`edit`：按规范化路径加锁，读共享、写独占；写完用临时文件加 rename。
- `bash`：独立进程组、独立 cwd 快照、独立输出通道；可并行，不参与文件路径锁。
- `cd`：只影响下一批调用；同一批共享批次开始时的 cwd。
- `todo`：只产生块，前端重放；不维护服务端共享可变列表。
- `web`：按标签页通道排队；同一会话的 search 串行，work 独立。

---

## 4. 批次与结果

一个 assistant 消息里的多个 tool call 属于同一批。

- 执行可以并发，结果可以按完成顺序到达。
- UI 可以按完成顺序显示。
- 模型上下文必须等整批全部结算后，一次性拿到所有结果。
- 结果按原始调用顺序提交，不按完成顺序。
- 成功、失败、超时、取消都要给每个 `call_id` 一个结果块。
- 用户打断：取消未完成项，结束回合，不发起下一次请求。

不能出现“第一个结果先回，模型先跑一轮，第二个结果后到”的上下文。

---

## 5. cwd

每个会话一个 driver。driver 持有该会话的 cwd。

- 新会话：等前端 `Post::Init` 交初始 cwd 和会话号。
- 旧会话：从会话行恢复 cwd。
- 模型 `cd`：改 driver 的 cwd。
- 用户 `/cdp`：改 driver 的 cwd。
- 工具执行：dispatch 时取 cwd 快照；已经在跑的调用不受后续修改影响。
- 下一批调用才看到新 cwd。

只有 bash 域里真正碰路径的工具读 cwd：

```text
bash, read, write, edit, cd
```

`alias` 只是命令展开，本身不读 cwd。`context`、`memo`、`todo`、`web` 都不读 cwd。

---

## 6. edit

只保留一个 `edit`：

```text
edit {
  type: "exact" | "mass",
  path: string,
  old?: string,
  new?: string,
  edits?: [{ line: int, text: string }]
}
```

- `type` 必填。
- `exact`：`old/new` 必填，要求唯一匹配。
- `mass`：`edits` 必填且非空，按行号替换。
- 另一组字段出现就报错。
- 两者共用路径解析、读文件、文件锁和原子写。
- `memo` 形状不变：exact 给 `atLine`，mass 给 `deletions`。
- `mass_edit` 的名字、模块、注册行全部删除，不留别名。

---

## 7. alias

alias 只服务 bash。

- 只展开整条命令的第一个 token。
- 只展开一层，不递归。
- 参数原样追加。
- 不支持变量、函数、条件、循环、多行。
- alias 命令不能为空，不能包含换行。
- `alias add` 先过 guard；bash 执行展开后的完整命令时再过一次 guard。
- alias 展开在 Rust 里做；bash 只执行展开后的命令。

alias 不能承载 HTML 解析、引擎选择、markdown 转换这类逻辑。HTTP-only 的外部 Python 脚本可以由 alias 起名，但逻辑在脚本里。

---

## 8. memo 与 todo

memo 只停留在 memo 的层面。

- 不做 persona 注入。
- 不做全大写 bootstrap memo。
- 不把 memo 当外部文件指针。
- `current` 绑块，`global` 跨会话。
- AI 只能写 `current`，不能碰 `global`。
- 人类通过 `/memo` 操作。
- 外部文档用 `read`，不用 memo 转发。

todo 是任务状态，不是 memo。

- 前端持有并重放。
- 服务端不维护共享可变列表。
- 与 memo 同目录只是归类，不共享存储和生命周期。

---

## 9. kairos、mcp、subagent

### kairos

- 一个工具，模式是 `now / after / every`。
- 纯 AI 工具，没有人类 `/kairos` 命令。
- 任务只在内存，进程停或会话关就取消。
- 结果作为普通块落库，wake 只报状态。

### mcp

- 一个静态工具 `mcp`。
- 不参与动态工具注册，不塞进 bash，不改 executor。
- 操作是 `list` / `call`，参数直接是 JSON。
- 第一版只做 stdio、同步、tools。
- `mcp-rmcp` 是可选 feature，tokio 只活在独立线程里。

### subagent

- `task` 只给主代理。
- `final` 只给子代理。
- 子代理禁止递归，roster 永远不含 `task`。
- 子代理默认不给 `web`，减少标签页。
- 子会话是普通 session，复用 agent loop、工具、存储和 relay。

---

## 10. 注册与执行

- `TOOLS` 是静态表。
- `roster` 按 profile 和 feature 过滤。
- `executor` 做名字查找、profile 门禁、参数解析、输出测量。
- MCP 不进入动态注册表；`mcp` 只是静态表里的一行。
- 工具不读数据库、不读配置文件、不碰全局状态；需要的输入全部从 `Env` / `ToolCtx` 进来。
- 注释只写规则，不写设计意图，不写过程。

---

## 11. 未决

- 文件路径锁的粒度：进程内还是跨进程。
- `bash` 与文件工具冲突时，是否接受“模型负责”的语义。
- 批次并发上限的具体数值。
- 子代理是否允许 `kairos`；当前不默认授予。
- `mcp` 在子代理里的可用性；当前按 profile 决定。
