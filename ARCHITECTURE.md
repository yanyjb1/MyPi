# MyPi 架构与耦合度报告

日期：2026-09-23 · 代码规模：~11.3k 行 Rust · 测试 317 全绿 · clippy 0

## 1. 分层与依赖边

```
   ┌──────────────────────────────────────────────┐
   │  tui/  （纯订阅端：事件 → Change → 画一帧）   │   tui → {server, ai, store, git}
   │   app ── keys ── editor ── path               │   app 持有 SessionState，但不碰轮数据
   │   layout ── view ── components/*              │   completion/leaf 独立状态机
   └───────────────┬──────────────────────────────┘
                   │ SessionEvent（协议输入）/ Change（协议输出）+ StreamView
                   ▼
   ┌──────────────────────────────────────────────┐
   │  server/  （服务端：会话服务，终端无关）      │   server → {agent, ai, store, entry}
   │   events    协议：SessionEvent / Change /     │   server → tui：0 条（刚扫描验证）
   │             StreamView                       │
   │   session   SessionState：按序消费事件，      │   无头可跑（headless 就绪）
   │             拥有流式槽/转录/pending/存储      │
   │   turn      轮线程机器：spawn_turn/           │   回调 → SessionEvent 翻译
   │             collect_turn/entries_to_context   │
   └──────┬──────────────┬──────────────┬─────────┘
          ▼              ▼              ▼
   ┌────────┐     ┌─────────┐    ┌────────┐
   │ agent  │ ──► │   ai    │    │ store  │ ──► entry（数据模型，叶子层）
   │ loop   │     │ client  │    │ sqlite │
   │ tools  │     │ config  │    └────────┘
   └────────┘     │ pricing │    git ── (无依赖)
     agent → ai   └─────────┘
     ai → 无（叶子层）
```

协议化后的数据流（单向）：

```
   键盘/粘贴/CLI ─┐
                  ├─► SessionEvent ─► SessionState::handle() ─► Change
   轮线程 (turn) ─┘                                       │
                                                          ▼
                                TUI 读 Change + stream_view()/snapshot() → 画一帧
```

实测边（`crate::` 引用扫描）：

| 源 | 目标 | 边数 | 评价 |
|---|---|---|---|
| ai / entry | — | 0 | 叶子层，干净 |
| agent | ai | 2 文件（loop_rs/tools） | 单向，正确 |
| store | entry | 1 | 数据模型依赖，正确（瑕疵 A 已随 entry 抽取消解） |
| server | agent/ai/store/entry | 各 1–2 | 服务端汇聚点，0 条指向 tui |
| tui | server/ai/store/git | 各 1–4 | 订阅端，经协议消费服务端 |

**关键改进（本轮）**：`AppEvent` 升级为 `server::events::SessionEvent` 协议；流式缓冲
（streaming/reasoning_buf/streaming_active/reasoning_done）从 App 字段迁入
SessionState；`drain_events` 退化为"事件泵进 session + TurnDone 时记价"。App 不再
直接改任何会话内脏——它只认 `Change` 和 `StreamView`。

## 2. 斜杠命令：单表驱动（本轮重构）

唯一权威源 `tui/path.rs::COMMANDS`（`CommandSpec{name, detail, args: ArgKind}`）：

- **注册**：新命令 = 表里加一行 + `App::run_command` 加一个 `cmd_*` 分支（编译器强制穷尽）
- **补全**：`ArgKind::{None, ModelId, Path}` 决定弹窗行为；`/cdp` 的 `Path` 参数已接线到文件路径补全
- **调度**：`submit` 按完整词查表（`lookup`），前缀不再误触发（修了 `/name` 吃掉 `/namexyz` 的旧 bug）；编辑器清理（clear/popup/goal_col/scroll）收敛到一处
- 原来散落的 `ARG_COMMANDS`、`PATH_ARG_COMMANDS`、200 行 if 链全部删除

## 3. 线程与共享状态模型

```
主线程（TUI 循环）                 后台轮线程
  App{...}                          spawn_turn:
  ├─ store（唯一写者，无锁）          ├─ Client clone（不可变）
  ├─ chat: Arc<Mutex<ChatContext>>   ├─ 读 cwd 快照 → BuiltinTools::new(cwd)
  ├─ cwd:  Arc<RwLock<PathBuf>> ◄───┤─ cd 工具写回槽（状态栏下一帧可见）
  ├─ cfg:  Rc<RefCell<Config>>       └─ 定稿后写回 chat，发 Commit 事件
  └─ client: RefCell<Client>
```

单写者纪律：DB 只在主线程写；轮线程只回传 entries。`Rc` 不出线程、`Arc` 才共享——编译器把关。

## 4. 意大利面评分（主观：耦合度 1-5，5 最面）

| 模块 | 分 | 说明 |
|---|---|---|
| ai/* | 1 | 叶子层；client 纯协议翻译 |
| agent/loop_rs | 1 | 纯循环 + trait 抽象，不认工具不认 UI |
| agent/tools | 2 | 自包含；与 ai 仅 types.rs 一处契约 |
| tui/path, keys, editor, layout, text, undo, history, paste | 1 | 全纯函数/纯状态机，零终端依赖，单测密集 |
| tui/components/* | 2 | 无状态渲染函数；chat 轻度依赖 ai::types |
| **tui/app** | **4** | 上帝对象：~1550 行、30+ 字段、触达全部下层（见瑕疵 B） |
| store | 3 | 本身简单，但反向依赖 Entry（瑕疵 A） |

## 5. 遗留瑕疵与建议（按优先级）

A. **store → tui 反向边**：`Entry` 是会话领域模型却住在 `tui::components::chat`。建议上提为独立模块（如 `src/entries.rs`），存储层即与 UI 解耦，未来做无头模式/CLI 时直接复用。

B. **app.rs 上帝对象**：命令分发已拆分（`cmd_*`），剩余三块仍可下沉：`CompletionController`（popup 状态+refresh/apply/tab 状态机）、`EventDrain`（AppEvent→transcript 映射，与 collect_turn 高度内聚）、`Resume` 流程。拆分后 app.rs 可回到"状态+循环"本职。

C. **`Entry::Error` 语义过载**：所有命令回显（`/model` 列表、`已命名`、`已恢复会话`）都借 Error 变体当"信息通道"。建议加 `Entry::Status{ text }`（渲染同 Error 但落库 kind 不同），resume 重建时不再把信息当错误。

D. **保留区三消费者手排优先级**：`App::reserved_height` 里 resume > popup > 空行的 if 链。当前规模可接受；若再加第四个浮层，改成 `[&dyn ReservedSource]` 优先级表。

E. **resume 重建只走协议消息**，UI 卡片（Entry）与协议消息（Message）各自从 entries 派生，同一数据源双向对称（collect_turn 为其逆）——设计成立，保持现状。

## 6. 测试与仓库卫生

- 全部测试不联网：网络层用假域名（example.test），无真实端点/密钥/模型名
- `.env`（含 API key）与 `models.yml`（本机端点）已从 git 历史彻底清除（孤儿提交重建），`.gitignore` 拦截；**旧 key 建议作废重发**
- 注释全英文；工具信息/报错/反馈全英文；过时叙述注释已删
