# MyPi 架构

一个终端编程 agent：会话服务（`server`，含网关 `ai`、执行引擎 `agent`、持久化 `store`）+ TUI（`tui`）。
规模：`src/` 约 2.64 万行 / 90 个文件；单 crate，edition 2024；`cargo test` 495 项全绿。

本文只写**现在成立的事实与规则**。规则由 `src/lib.rs` 里的 `mod architecture`
测试强制：写错一条依赖，`cargo test` 直接红。

---

## 1. 构建矩阵

```bash
cargo build                    # 默认：含联网工具（fetch / search / browser）
cargo build --no-default-features   # 剔除整个 web 域（无 CDP、无 HTML→MD、无 URL 解析、不起浏览器）
cargo test                     # 默认特性
cargo test --no-default-features    # 联网相关测试随之消失（少 28 个）
```

`web` 是唯一的功能开关，管三件事：`src/web/` 模块、`tungstenite` /
`html-to-markdown-rs` / `url` 三个依赖、以及名册里的三个联网工具。
关掉之后模型看不到 `fetch`/`browser`/`search`，硬调也会得到
`tool ... is not compiled in` 而不是静默成功。

---

## 2. 分层（唯一的硬规则：只许向下依赖）

```
        ┌────────────────────────────────────────────┐
        │ tui/   表面：事件 → Change → 画一帧          │  tui → 任何
        │ app keys editor view layout completion/     │
        │ transcript/ components/ theme/              │
        └───────────────┬────────────────────────────┘
                        │ SessionEvent（进）/ Change（出）+ StreamView
                        ▼
        ┌────────────────────────────────────────────┐
        │ server/  服务：会话状态机，终端无关           │  server ↛ tui
        │ events session turn compaction profile      │
        └──────┬──────────────┬───────────────────────┘
               ▼              ▼
        ┌───────────┐   ┌──────────┐   ┌───────────┐
        │ agent/    │──▶│ ai/      │   │ web/      │  agent → web（工具派发）
        │ loop tools│   │ client   │   │ fetch     │  web ↛ agent/server/tui
        │ guard art.│   │ config   │   │ search    │
        └───────────┘   │ types    │   │ browser   │
                        │ pricing  │   └───────────┘
                        └──────────┘
        ┌──────────────────────────────────────────────────────────┐
        │ 叶子：entry（数据模型）store（SQLite）grouping ansi git xdg │  谁也不许指向上层
        └──────────────────────────────────────────────────────────┘
```

规则表（`src/lib.rs::architecture::FORBIDDEN`，逐条是「该模块**不得**出现
`crate::<这些>`」）：

| 模块 | 禁止指向 |
|---|---|
| entry / grouping / ansi / git / xdg / store / ai | agent, cli, server, tui, web |
| agent | cli, server, tui |
| web | agent, cli, server, tui |
| server | **tui** |

**为什么 server ↛ tui 是要害**：`server` 是服务，不是终端的一部分。它只认
`SessionEvent`（输入）/ `Change`（输出）这套协议，所以第二个订阅端（Telegram
桥接、无头驱动）只要会收发协议就能接进来，不需要碰会话内脏，也不需要动 `tui`
一行。这条边一旦破，桥接就得跟着 TUI 的类型走——所以它单独有一条测试
`the_server_never_learns_about_the_terminal`。

测试怎么判：扫 `src/**/*.rs` 的 `crate::` 引用；剔除 `#[cfg(test)] mod` 块
（测试代码跨层是合法的）与整行注释（文档里**提到**别的层不算依赖）。

---

## 3. 会话协议

```
键盘/粘贴/CLI ─┐
               ├─▶ SessionEvent ─▶ SessionState::handle() ─▶ Change
轮线程 (turn) ─┘                                          │
                                                          ▼
                            TUI 读 Change + stream_view()/transcript() → 画一帧
```

- `SessionEvent`：`Delta` / `ReasoningDelta` / `ToolStart` / `ToolFinish` /
  `Error` / `TurnDone` / `Done` / `Submit` / `NameMarker` / `SetCwd` / `Compaction`
- `Change`：`Transcript` / `Stream` / `TurnDone` / `ToolActivity` / `Session` / `None`

轮线程（`server::turn::spawn_turn`）只回传事件，不碰 DB；DB 只有主线程写。

---

## 4. 落盘契约：逐字节复现

数据库的唯一职责是**把当时的对话原样复现**：

- 一轮的条目在 `TurnDone` 时**一次性**写入（`sessions` + `entries`，树形
  `parent_seq` + `leaf` 指针）；
- `entries_to_context` 由条目重建协议消息，与模型当时收到的 wire 形状逐字节一致
  （多调用合成一条 assistant、`args` 原样回放、reasoning 不进协议）；
- 空回复有唯一占位符 `entry::EMPTY_REPLY`：**落盘与实时上下文共用同一个字符串**，
  否则重启后回放的字节就与当时发出去的不同（前缀缓存全冷，模型看到的是一份它没发过的历史）；
- `leaf` 是用户状态（回溯、`set_leaf(None)` = 回到根），**跨重启存活**；
  迁移只在列刚被 `ALTER` 加上的那一次执行，不重复回填。

覆盖这些的测试：`store::tree_tests::*`（含 `a_stored_round_replays_byte_identically_after_a_reopen`）、
`server::turn::tests::a_whole_conversation_replays_byte_identically`。

---

## 5. 联网域（feature = "web"）

```
agent/tools.rs ──dispatch──▶ web/{fetch,search,browser}/engine.rs ──▶ providers/ ──▶ utils/session.rs ──▶ utils/cdp.rs
```

- 每个工具一个目录：`engine.rs` 路由 + `providers/` 实现；
- 浏览器进程与标签页由 `utils/session.rs` 统一持有（attach 优先、launch 兜底，
  `work` 标签跨调用存活，search/fetch 用一次性标签）；
- CDP 的 socket、重连、`dom_html` 兜底都在 `utils/cdp.rs`。

**选哪个浏览器由 config.yaml 决定**（环境变量优先，便于测试与并行会话）：

```yaml
browser:
  bin: /usr/bin/chromium        # 留空 → $MYPI_BROWSER_BIN → /opt/helium/helium
  port: 9222                    # 接已在跑的实例；留空 → 发现同 profile 的活实例 → 启动
  profileDir: ~/.local/share/mypi/browser/profile   # 登录态、扩展
  headless: true
  userAgent: ""                 # 留空 → 内置的稳定版桌面 UA
  extraArgs: ["--proxy-server=http://127.0.0.1:8080"]
```

生效时机：`bin`/`headless`/`userAgent`/`extraArgs` 在下一次**启动**浏览器时生效；
`port` 每次 ensure 都读。浏览器进程活着时改配置不会热重启它。

---

## 6. 模块地图（行数，2026-09-25）

> 服务端要拆成 daemon、前端走 socket 的设计见 **[SERVER.md](SERVER.md)**（一页纸，未动代码）。

| 模块 | 行数 | 内容 |
|---|---|---|
| `server/` | 11,006 | **服务端全部**：`events`（协议）、`session`（状态机 + facade）、`turn`（轮线程 + 上下文重建）、`hub`（多会话宿主）、`compaction`、`profile`、`log`（内存日志）、`store`（SQLite + 回合请求头）、`entry`（协议数据模型）、`ai/`（网关：client/types/config/pricing）、`agent/`（执行引擎：loop/tools/artifacts/bash_guard） |
| `tui/` | 16,188 | 前端：`app`、`keys`、`zone/main/{history,input,reserved}`、`input/editor`、`statusline`、`theme`、`session/{loop,signal}` |
| `web/` | 2,319 | `fetch` / `search` / `browser` 三域 + `utils/{cdp,session,html,url}` |
| `grouping` `xdg` `git` `ansi` `cli` | 668 | 叶子层（块分组、平台目录、git 状态、ANSI 剥离、命令行） |

**2026-09-25 搬家**：`entry` / `store` / `ai` / `agent` 从 crate 根目录收进 `server/`。
前端（含 TUI）只认 `mypi::server`，服务端换内部结构不再牵动别人调用点。
分层规则同时改成**路径前缀对**（`lib.rs` 的 `architecture` 测试），
所以文件搬进子模块后规则仍然指得准（例如 `server::agent` 不许引用 `server::session`）；
另有一条兜底：`server/` 下任何文件都不许点名 `tui`。

config.yaml 的 schema（含 `tools` / `browser` / `compact` / `theme` / `profile` / `streaming`）
**只在 `server::ai::config` 定义**：它不认上层类型，消费方向下 import。

---

## 7. 线程与共享状态

```
主线程（TUI 循环）                 后台轮线程
  App{...}                          spawn_turn:
  ├─ session: Session               ├─ Client clone（不可变）
  │   ├─ store（唯一写者，无锁）      ├─ cwd 快照 → BuiltinTools::new(cwd)
  │   ├─ chat: Arc<Mutex<Ctx>>  ◀───┤─ cd 工具写回 cwd 槽
  │   ├─ cwd:  Arc<RwLock<PathBuf>>  ├─ 工具事件 → SessionEvent
  │   └─ client: RefCell<Client>     └─ 定稿后写回 chat
  └─ block_cache / completion / editor
```

`Rc` 不出线程、`Arc` 才共享——编译器把关。轮线程只发事件，落盘在主线程。

**多会话（`SessionHub`）**：一个进程开 N 个会话，每个会话**自己一条 SQLite 连接**
（WAL：读不阻塞、写排队），自己的转录、cwd、模型、流式槽、事件通道。
数据隔离是结构性的——每张表都以 `session_id` 为主键的一部分，没有代码路径能跨过去。
并发来自「一个回合一个线程」：两个会话可以同时生成，各自的事件只能回到自己的 id。
宿主本身**不是线程**：它和会话一样由**一个线程**独占驱动（今天就是主线程），
所以不需要锁；SQLite 负责把写者排好队。

---

## 8. 测试约定

- 全部离线：网络层用假域名（`example.test`），测试里没有真实端点/密钥；
- 断言**可观察行为**（协议字节、落盘内容、渲染出的行），不钉实现细节；
- 渲染类测试用 `ratatui::backend::TestBackend` 画真帧再读缓冲
  （注意宽字符会占两格，比较前去掉填充空格）；
- 交互语义（如「光标贴着标记时一次退格删整块」）写进对应模块的测试；
- `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check` 应当是绿的。

---

## 9. 已知取舍

- **bash 许可区是分类器，不是沙箱**：它按命令位置（而非子串）判定 `rm`/`mv`/`find -delete`/
  凭据写入/关机类，误杀已收敛，但挡不住铁了心的对手。
- **聊天流没有 body 级超时**：只有 connect / 等响应两个超时。长回答不能被总时长砍断，
  代价是「响应头到了之后卡死」仍会挂住轮线程（Esc 只在收到 delta 时轮询）。
- **工具能写任意路径**：`browser read/screenshot` 的 `path` 不受许可区约束，
  与 bash 的区外禁令不一致。
- **无 CI**：`clippy 0 警告` 是手跑出来的结论。
