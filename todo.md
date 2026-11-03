# TODO

整个项目剩下的活。收尾期，按能开工的先后排。

施工完毕后，务必更新本文档。

## 一、需要授权：config 的主题键

`src/server/config/schema.rs` 里五个键（`theme.name`、`accent`、`gold`、`black`、`muted`）已经没有读者：前端不再问它们，色系只从启动随机取一条。删键是域外改动，等点头。

## 二、历史区

### 待设计

- wake 提醒的样式。现在一条 wake 一个块、一条一个 `stream` 帧，画成一行 notice（`live.rs` 的 Wake 分支）。
- 每类工具的流式动画。历史区只提供位置，运行中的卡片已在。

### 其余

- 工具专有渲染缺口，仍走兜底卡片：`context`（输出是表，表段已具备）、`memo`、`kairos`、`mcp`、`alias`。`final` 不渲染，`task` 随子会话面板一并定。
- `/usage` 面板的横向对齐没有读者。
- 补全的 pools 拿到一次就不再刷新。
- 行级流式增量：现在按段增量（闭合段缓存、开段重算），段内还没到「只动新那一行」。
- 表格块跳过解析层（列宽依赖宽度），resize 时整块重排。
- 转交的兜底：尾块数与结构区间数对不上时，退回「清空可变区 + `call blocks` 重取」，会闪一帧。

## 三、机制层的小坑

- `name::RESIZE` 有人喊没人听：三个子区都不处理，尺寸实际走 `deliver_event` 的参数。
- 行高握手（`request_height` / `assign` / `on_height_changed`）挂在通用 `SubZone` 契约上，是主区的私事。
- `HistoryZone::request_height` 用 `u16::MAX` 当哨兵。
- 三个子区都丢弃 `on_height_changed` 的返回值。

## 四、已知代价（记录，暂不修）

- 可见槽的每一帧都复制一次行。
- `run` 每帧无条件重绘，`Term::poll` 固定 16ms。
- 转交之后仍要为同一区间发一次 `call blocks`，只为取回流上没有的 `usage` 与 `first`。

## 五、等定夺

旧 `app-目标形态` / `app-施工注意` 里未落地、也没进任何 README 的约定：

- 登记检查：zone 名字唯一、同一个键不得被两个 zone 声明，违反即 panic。现在不检查，重复声明按表序靠前者赢。
- 按键由声明决定，而不是由显示权决定。现在只把键交给持有显示权的 zone。
- `Zone::render(&mut Frame)`：zone 自己画、自己设光标。现在是 zone 返回行、app 开帧。
- 事件种类枚举与后端出口一一对应。现在是 `class / name / content` 三元组。
- memo 只活在交接窗口。现在由 app 长期持有。
