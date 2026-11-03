# TODO

整个项目剩下的活。收尾期，按能开工的先后排。

## 一、接线：`src/main.rs`

仍是上一波后端改造之前的调用，二进制编不过，整个 TUI 因此没法跑。要改的调用：

- `Config::load` 还在，但没有 `cfg.app.*` 这棵树；所有取值改走 `Config::get(key, profile)`。
- `Api::open(db, cfg)` 现在是两个参数，没有 `batch`。
- `time::init(...)` 的时区从 `Config::get("config.server.platform.timezone", …)` 取。
- IPC 种类从 `Config::get("config.server.platform.ipc", …)` 取。
- `server::run` / `server::bind` / `server::serve` 与 `Api::open` 的新签名对齐。

接完这一处才能冒烟；之后 history 的 resize、流式、折叠才有得测。

## 二、上游：mailbox 改成独立单槽

定的形态：一条独立通道，只有 `write` 与 `del`；单槽，整块覆盖，`del` 清空；没有排队、没有取走、没有逐条撤销。前端判断忙：`submit` 过且该回合的结构回执（`ranges`）没到。

四处要动：

1. `loop/session/mailbox.rs`：现在是 `wake` 与 `user` 两条队列，改成单槽写入/清空。
2. `loop/session/book.rs::deliver`：忙时不再 `push_user` 排队，改为覆盖写入并回 `Ok(None)`。
3. `api/subscribe/submit.rs`：拆掉 `relay().busy()` 那道闸与 `Ok(None)` 的拒绝；`submit` 的答复要能表达「这一句没有开回合」。
4. `relay.opened` 在投递进信箱时不记账；另需一扇 `del` 的门（新的 post，或并入 `interrupt`）。

## 三、需要授权：config 的主题键

`src/server/config/schema.rs` 里五个键（`theme.name`、`accent`、`gold`、`black`、`muted`）已经没有读者：前端不再问它们，色系只从启动随机取一条。删键是域外改动，等点头。

## 四、历史区

- 每类工具的流式动画：用户另做设计，历史区只提供位置（运行中的卡片已在）。
- 工具专有渲染缺口，仍走兜底卡片：`context`（输出是表，表段已具备）、`memo`、`kairos`、`mcp`、`alias`。`final` 不渲染，`task` 随子会话面板一并定。
- `/usage` 面板的横向对齐没有读者。
- 补全的 pools 拿到一次就不再刷新。
- 行级流式增量：现在按段增量（闭合段缓存、开段重算），段内还没到「只动新那一行」。
- 表格块跳过解析层（列宽依赖宽度），resize 时整块重排。

## 五、机制层的小坑

- `name::RESIZE` 有人喊没人听：三个子区都不处理，尺寸实际走 `deliver_event` 的参数。
- 行高握手（`request_height` / `assign` / `on_height_changed`）挂在通用 `SubZone` 契约上，是主区的私事。
- `HistoryZone::request_height` 用 `u16::MAX` 当哨兵。
- 三个子区都丢弃 `on_height_changed` 的返回值。

## 六、已知代价（记录，暂不修）

- 可见槽的每一帧都复制一次行。
- `run` 每帧无条件重绘，`Term::poll` 固定 16ms。

## 七、等定夺

旧 `app-目标形态` / `app-施工注意` 里未落地、也没进任何 README 的约定：

- 登记检查：zone 名字唯一、同一个键不得被两个 zone 声明，违反即 panic。现在不检查，重复声明按表序靠前者赢。
- 按键由声明决定，而不是由显示权决定。现在只把键交给持有显示权的 zone。
- `Zone::render(&mut Frame)`：zone 自己画、自己设光标。现在是 zone 返回行、app 开帧。
- 事件种类枚举与后端出口一一对应。现在是 `class / name / content` 三元组。
- memo 只活在交接窗口。现在由 app 长期持有。
