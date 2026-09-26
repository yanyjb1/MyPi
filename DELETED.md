# DELETED.md — 删除记录

## keylink.rs（借键旧机制，整文件删除，219 行）

- `KeyLending::declares/open/lends`：输入区持有外借清单——清单所有权
  归错了人，删除。
- `expose()` + 契约冲突 panic：声明制下没有"借了不存在的键"需要
  panic 的场景（借方没声明的动作静默退回），删除。
- `Lent::Consumed/Observed`：路由返回值，广播制下无路由，删除。

替代机制（子区借键规范化）：
- 借出清单 = 保留区所有者的 `Claim::accepts()`（借方自己声明，精度
  到具体 Action），机制在 `components/reserved.rs`，未动。
- 输入区 hang=true 时把翻译好的 ↑↓Tab 打包 `ToReserved::Lend`，
  `InputZone::pending_lend` 暂存；MainZone 投递给 `ReservedZone::lend`
  （Zone 代传达是 Rust 所有权需要——兄弟字段互拿不到 &mut——不是
  语义设计）；`ReservedArea::offer` 按 claimant 声明自取，退回的
  归输入区。
- `ReservedZone::claims` 注册表已就位（现为空）；补全服务重写为
  Zone 时 `attach` 入册。hang 触发点未实现，现恒 false。

破坏性重构中删除的模块与其承载的功能。此处只记录"删了什么、原本干什么"，
方便日后按新架构（APP → Zone → SubZone → 组件）重写。

## 删除时间线

- 2026-09-25 破坏性重构第一轮（Zone 契约落地时）。

## src/tui/view.rs（整个文件，515 行）

旧渲染大杂烩：`ViewState`（15 字段借用胶水）+ `draw()` 单函数包办全屏。
被"Zone 自渲染"（`zone/main/mod.rs` 的 `Zone::render`）取代。删除的功能清单：

1. **竖向布局切分**：history / gap / container / reserved 四块矩形切分
   （`Layout::vertical` + `tlayout::Layout` 约束）。→ 职责并入 Zone 行高仲裁
   （`MainZone::arbitrate`）。
2. **状态栏绘制**：`s.status.render()` 取一行画在容器顶。→ 状态栏组件
   本身保留（`tui/statusline/`），挂载点待 Zone 接管后重接。
3. **输入容器绘制**：`input::render(InputSpec{...})` + 光标 (row,col) 定位。
   → 待重写：InputZone 的 `render_rows`（现为空 TODO）。
4. **历史区绘制**：BlockCache 反向走窗（`window_from_bottom`）+ live tail
   （thinking/working 标签、流式内容）拼接 + 滚动裁剪（cut_bottom/keep_from）。
   → 待重写：HistoryZone 的 `render_rows`（现为空 TODO）。
   注意两条铁律：
   - live tail 行数必须在切窗**之前**预留（否则满屏时流式内容不可见）；
   - 窗口定位取走窗结果的**顶部**（取底部会导致滚轮时画面不动）。
5. **保留区绘制**：idle 一空行 / holder 预渲染行 / resume picker legacy。
   → 待重写：ReservedZone 的 `render_rows`（现为空 TODO）。
6. **硬件光标定位**：容器内 (row,col) → 终端 Position，带边界钳制。
   → 待重写：谁渲染输入区谁报光标位。
7. **`hard_wrap` / `hard_wrap_for_test`**：按显示宽度硬折行（CJK 宽字符
   感知），仅测试用。→ 若重写渲染需要，从 git 历史捞（本文件提交历史）。

## src/tui/chat/（整个目录）

### commands.rs（~200 行）
斜杠命令表 + 实现：`/q` `/cdp` `/name` `/model` `/switch` `/compact`
`/profile`，`run_command` 分发器，`build_resume_items`。全部 `impl App`。
功能本身保留需求：命令表驱动、参数候选补全（ArgKind）。→ 将来以
"命令属于输入子区的补全管线"身份重写，不进 APP。

### tree.rs（~110 行）
`resume_confirm()`：会话恢复（读 store → entries/meta/effective_name/
cwd_history → 四路恢复 transcript/context/cwd/name）+ 首轮 cwd 注记
（pending_cwd_note）。`tree_navigate_to()`：树导航跳转。
→ 将来 resume 以独立 Zone 重做（ZoneId::Resume 已在契约中留位）。

## App 旧字段/方法（原 app.rs 771 行 → 现 ~100 行 Router）

- `apply()` 路由链（modal > editor > app-level 三级 match）→ 由
  `Ownership` 账本 + `Zone::deliver` 下发取代。
- `editor_handles` / `editor_action` / `EditAction`：编辑键语义分派 +
  单一 epilogue（finish_edit 后统一 refresh_completions，Nothing 除外）。
  → 待 InputZone 重写编辑语义时恢复该模式（epilogue 单点值得保留）。
- `key_context()`：翻译键所需上下文（popup_open/streaming/browsing…）。
  → 待补全管线归位 InputZone 时重接。
- `resolve_reserved` / `reserved_draw` / `reserved_height`：每帧保留区
  仲裁三件套 → `MainZone::arbitrate/broadcast` 取代（App 不再仲裁）。
- `submit()`：命令分发 vs 正文提交、busy 拦截、input_history push、
  滚动重置回底部。→ 待重写于输入子区/会话服务。
- `refresh_env` / `git_at` / `env_cwd`：git 快照事件驱动限频。
- `prime_statusline` / `tick_statusline` / `usage_snapshot`：状态栏
  广播（ModelChanged/Usage/Tick/Activity 边沿）。→ 状态栏组件保留，
  广播源待接（每帧一次 Tick 的心跳逻辑可复用）。
- `set_workspace` / `pending_cwd_note` / `display_name`：工作区迁移。
- KeyLending 接线（`keylink::expose` + CompletionClaim.accepts()）：补全
  弹窗借键。→ 机制保留（`tui/keylink.rs` 未删），调用点待补全管线归位。

## zones_impl.rs 旧版（wants/handle/Cascade 时代）

- `Zone::wants` 谓词路由表、`Cascade` 跨区效应枚举（LayoutDirty 等）：
  被"所有权下发 + Handover 交接"模型取代，不再有跨区效应广播。
- `wheel_zone()` 行命中测试：滚轮不再按行路由，滚轮语义整体归历史子区。

## 子区事件声明制（RawEventKind）

Zone 从 APP 接过全部物理事件（键/粘贴/滚轮），只转发给**声明了解释
该事件类别**的子区。声明在 `SubZone::accepts()` 里，是注册的一部分；
声明粒度是事件类别（`RawEventKind::{Key, Paste, ScrollUp, ScrollDown}`），
不是具体键位——拿到 `Key` 的子区看到原始 KeyEvent（修饰键原样携带），
哪个键触发什么行为是子区内部的解释，Zone 不猜、不默认、不翻译。

- 历史区：声明 `Key + ScrollUp + ScrollDown`。Ctrl+T/Ctrl+O 与滚轮回滚
  都在自己 `interpret` 里解释原始事件。
- 输入区：声明 `Key + Paste`。编辑键语义走私有 `semantics::translate`。
- 保留区：声明空 `&[]`。它不需要键位触发，事件由输入区联动唤起。

滚轮归一化在 `keys::normalize_mouse`：只认 ScrollUp/ScrollDown，鼠标
移动和左右键直接丢弃（用户决策：现阶段只管滚轮）。loop.rs 的
`app.main.history.wheel_step` 特判已删——滚轮和键位同路下发。

## 键位层级迁移（keys.rs 拆分）

原 `tui/keys.rs`（724 行）是"物理捕捉 + 语义翻译"混住的屎山。已拆成：

- `tui/keys.rs`（现 ~50 行）：**全局只管捕捉**。`RawEvent::Key{key}` /
  `RawEvent::Paste(String)`，`normalize(KeyEvent)` 原样携带修饰键，零语义。
  Ctrl+V 兜底粘贴已删（用户决策：粘贴要 Shift，只走 bracketed paste）。
- `tui/zone/main/input/semantics/`：**语义归最终消费者**。原 Action 枚举
  （编辑/历史/选择器/树/补全全部变体）、KeyContext、translate/translate_with
  及全部键位测试原样迁入。历史区的 Ctrl+T/Ctrl+O 由 HistoryZone 直接解释
  原始键（它自己的语义不过任何上级）；编辑键语义归输入子区的
  semantics::translate。Zone（MainZone::deliver）只转发 RawEvent，不解释。
- `keylink.rs` 已删（见文首）。`Claim::accepts` / 补全弹窗借键清单：
  改指向 `zone::main::input::semantics::Action`，机制归
  `ReservedZone::lend` / `ReservedArea::offer`，组件侧未动。

## 状态栏为什么是空的（不是坏了，别慌）

新输入子区接管状态栏之后，如果你看到 `+--` + 图标 + 一长串横杠 + `0%:0` 这种
只剩下空壳的样子，**那不是渲染坏了、也不是状态栏丢了**：

- **现象**：模型名、文件路径、git 分支、花费、上下文用量全是空的。
- **原因**：没有任何地方调 `InputZone::notify(StatusEvent::…)`。部件都还在、
  注册表还在、渲染还在，只是**没人喂事件**，所以每个部件都画自己的空态。
- **TUI 侧已完成**：`InputZone` 持有 `StatusLine`（容器顶边就是它），通道是
  `InputZone::notify(&StatusEvent)`；部件自己订阅、自己更新、自己画，输入区
  不解析任何事件含义。
- **缺的是上游事件源**（会话层 / server 侧，正在重构）：`ModelChanged` /
  `WorkspaceChanged` / `CwdMigrated` / `SessionRenamed` / `Git(..)` /
  `Usage(..)` / `Tick` / `ActivityStarted|Stopped` 要在对应时机发出。
- **可参照**旧 app.rs 的 `prime_statusline` / `tick_statusline` / `usage_snapshot`
  （每帧一次 `Tick` 的心跳逻辑可复用），见本文件「App 旧字段/方法」一节最后一段。
- **纪律**：不要为了让状态栏“看起来有内容”而在 TUI 里塞假数据；空态就是空态，
  等事件源接上自然就满了。

## 历史区为什么也是空的（同样不是坏了）

如果你启动后发现**历史区一片空白**（输入框能打字、状态栏是空壳）：

- **现象**：转录区没有内容，滚轮滚不动（本来也没东西可滚）。
- **原因**：`HistoryZone::push_entry` / `replace_transcript` **目前没有任何
  调用者**——会话事件（server 侧）还没接到 TUI 的历史子区。渲染、缓存、
  滚窗、折叠开关全都就绪了，只是没人往里送数据。
- **TUI 侧已完成**：`push_entry(e)` 逐条追加（追加不必动「代」）、
  `replace_transcript(v)` 整体替换（自动把「代」+1，缓存据此丢弃旧行）、
  `entries()` 只读。两件事都别绕过这几个入口直接改里面的 Vec。
- **接线要点**（将来 server 侧重构时）：
  - 每收到一条会话事件就往 `push_entry` 送一条 `Entry`；
  - resume / 树跳转这种「整体换一支转录」必须走 `replace_transcript`，
    **且不要**自己维护序号——缓存按块的 `Range`（从哪条到哪条）认身份；
  - 流的中间态（半截回复）现在还没设计，等流式方案定了一起做（两条
    铁律见本文件 `view.rs` 那节：live tail 的行数要在切窗**之前**预留、
    窗口定位取走窗结果的**顶部**）。

## 服务端的落盘契约（这一轮定下的）

### 1. `rounds` 表：请求头必须落库

`entries` 存的是**转录**（模型上下文里的对话），但「这次请求到底长什么样」不在转录里：
系统提示词、工具手册、模型 id、端点、token 上限。这些以前只存在于**本地配置和 profile**
（启动时重新解析），所以换台机器、换个 UI、改了 config，就复现不出当时那份请求。

现在每轮结束（或出错）时，entries 和一行 `rounds` **在同一个事务里**写下去：

| 列 | 是什么 |
|---|---|
| `model` / `base_url` / `protocol` | 这轮跟谁、在哪、用什么协议说话 |
| `system` | 系统提示词**原文** |
| `tools_json` | 工具手册**原文**（发给模型的那个数组） |
| `max_tokens` | 配置的 token 上限（每次请求的推导值由同一个函数重算，确定性一致） |
| `stop_reason` | `stop` / `length` / `tool_calls` / `interrupted`；**NULL = 网关没回答就死了** |
| `first_seq` / `last_seq` | 这轮写了哪几条 entries |

**不存 API key**（凭据不进库）。

### 2. `Session::replay_round(session_id, round_seq) -> Replay`

只读数据库，把那一轮的请求重建出来：`system` / `tools_json` / `model` / `base_url` /
`messages`（那一轮结束时的上下文，也就是下一次请求的前缀）/ `body`（请求体 JSON）。
`body` 由**实时客户端用的同一个序列化函数**产出（`ai::client::body_json`），所以不可能漂移。

读不出来就**报错**，不许静默跳过：少一条 entry 的请求「看着挺像」，那比没有更坏。

验证过的性质（见 `server::turn` 的假网关用例）：拿真发出去的请求和从库里重建的比，
**前缀逐字节一致**；工具轮里 `args` 原样、工具结果原文、`assistant(tool_calls)` 与
`tool(result)` 的形状都一致。

### 3. 什么落、什么不落

| 内容 | 落盘？ | 为什么 |
|---|---|---|
| 用户消息、回复、思考、工具请求/结果 | 落 | 进入过模型上下文 |
| 系统提示词、工具手册、模型、端点 | 落（`rounds`）| 决定请求的字节 |
| 部分回复（用户打断 / 网络中断）| 落 | 卡到最后一个字也要能复现 |
| `Entry::Error`（中断/超时/重试这类本地叙述）| **不落** | 只给用户看，与请求字节无关 |
| `Entry::System` 通知（切模型、压缩提示）| **不落** | 同上；老注释说「像其他条目一样持久化」是假的，已改 |

### 4. 流式交付开关

- `config.yaml → app.streaming: immediate | buffered`
- 环境变量 `MYPI_STREAM_MODE`（调试用，写错**不生效**而不是悄悄退回默认）
- 代码里 `Session::set_stream_mode(...)`（下一个回合生效）

解码永远只在服务端发生一次；开关只决定**什么时候**把已解码的正文交出去。
`buffered` = 整轮一次性交付，最终状态与 `immediate` 逐一相同（有测试）。

### 5. 受限客户端的一个值：`Session::status() -> RunState`

`Idle` / `Thinking` / `Replying` / `Tool{intent}`，由流式槽推导。电报这类客户端渲染不了
转录，也不该去解工具参数才能说一句「正在调用工具」。

### 6. 这一轮顺带修掉的两个真 bug

- **报错路径不写回上下文副本**：库里存了半截回复，内存副本里没有 → 下一次请求比库里少一条，
  复现对不上。现在 `Err` 分支也写回（复用同一个占位规则）。
- **`delete_session` 漏删 `rounds`**：会话删了请求头还在，新会话复用同一个 id 会读到别人的。

### 7. 行为变化（故意的）

请求体 JSON 的**键序**变成字母序（以前按结构体声明顺序）。内容一个字节没变，只是为了让
「实时发的 body」和「从库重建的 body」能直接比较 —— 这正是复现测试的前提。

## 多会话、重试日志、目录搬家（2026-09-25）

### 1. 一个 server，多个会话

`server::SessionHub`：

```rust
let mut hub = SessionHub::new(db_path());
let a = hub.open_new(spec_for_project_a)?;   // 自己的连接、转录、cwd、模型
let b = hub.open_new(spec_for_project_b)?;
hub.submit(a, "画个图");
hub.submit(b, "画别的");                      // 两个回合同时在飞
for (id, changes) in hub.drain_all() { /* 事件带着自己的会话 id 回来 */ }
```

- **隔离是结构性的**：`entries` / `rounds` / `cwd_history` / `artifacts` 全都以
  `session_id` 为主键的一部分，没有任何路径能跨会话读。
- **每个会话一条 SQLite 连接**（同库，WAL）：读互不阻塞，写排队 —— 两个会话同时生成、
  同时落盘是安全的。
- **并发来自「一个回合一个线程」**；宿主本身不是线程，由**一个线程**独占驱动
  （今天就是 UI 主线程），所以不需要锁。
- 测试里两个会话各起一轮、同时跑、同一个库文件同时写，断言转录/请求头/cwd/复现各是各的。

### 2. 重试与内存日志

- 常用设定：**总共 3 次尝试**，退避 400ms → 800ms（×2，上限 8s，带 ±25% 抖动）。
- 只重试**瞬态**失败：超时、socket 错误、连接失败、DNS、408/409/425/429、5xx。
  坏 URL / 协议错 / TLS / 4xx（上列之外）不重试 —— 重试也是白花钱。
- **已经吐出内容就不许重试**（重试会把开头再放一遍）。规则：没吐过才许重试；
  吐过就按「中途死亡」处理 —— 半截回复落盘，这一轮结束。
- 每次失败、每次放弃都写进 `server::log` 的**内存环形日志**（容量 256，不落盘）：
  `log::recent(n)` / `log::by_scope("模型名")` 读。**不落盘是故意的**：重试是我们自己的
  行为，不是模型看到的东西（与 `Entry::Error` 同一条规矩）。
- 策略现在是**代码里的默认值**（`RetryPolicy`），还没接 config.yaml：按你说的先照常用的来。

### 3. 目录搬家

`entry` / `store` / `ai` / `agent` 从 crate 根目录收进 `server/`：
`mypi::server::{entry, store, ai, agent, events, session, turn, hub, log, compaction, profile}`。
前端只认 `mypi::server`。`tui/session/db.rs` 里的库路径与旧库迁移并入 `server::store`。

分层规则（`lib.rs` 的 `architecture` 测试）同时升级成**路径前缀对**，
并加了一条守卫测试；文件搬家后再想越层引用会直接测失败。

## 设计与排查文档的索引

- **SERVER.md** —— 服务端拆成 daemon + 前端协议的一页纸设计（进程边界、消息表、线程、打断、背压、落地顺序）。
- **ARCHITECTURE.md** —— 分层规则、模块地图、线程与共享状态。
- **本文件** —— 删了什么、为什么；各类「为什么是空的 / 为什么不落盘」的行为契约。
