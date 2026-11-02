# main TODO

## 上游待接线：mailbox 改成独立单槽通道

决定的形态：

- mailbox 不挂在 `submit` 的时机上，是一条独立通道，只有两个操作：`write` 和 `del`。
- 信箱是单槽，内容是整块：`write` 覆盖之前的内容，`del` 清空全部。没有排队、没有取走、没有逐条撤销。
- 前端判断忙：`submit` 过、且该回合的结构回执（`ranges`）还没到 = 忙。忙期间的 `submit` 自动改投 mailbox，不再尝试开回合。

要改的地方（上游）：

1. api 拆掉 `subscribe/submit.rs` 里 `relay().busy()` 那道闸，`Ok(None)` 不再当 busy 拒。
2. `relay.opened` 在"投递进信箱"时不记账——没有回合号可记。
3. `submit` 的答复要能表达"这一句没有开回合"。

属于上游接线，不在 TUI 这一波内。

## 等待别的域

- `src/main.rs` 仍是上一波后端改造之后的旧调用（`Config::app`、`Api::open` 的参数），二进制编不过；TUI 因此还不能跑起来做冒烟。
- `run` 仍是每帧无条件重绘；"有帧或有输入才重绘"要在那一层改。

## 留着的待定

- 流式块每来一个件整块重排一次，没有按段增量。
- resize 的保留切点重排还没建：`Viewport::on_resize` 只把锚点行钳进新高度。
- 常驻圈之外的成品不按距离裁，只按缓存预算淘汰。
- 思考链不再钉在底部，随块就地渲染。
- `/usage` 的横向对齐没有读者；补全的 pools 拿到一次就不再刷新。

## 工具渲染的缺口

后端名册十四个。已有专属：`read`、`cd`（单行，不进卡片）、`write`、`edit`、`bash`、`todo`。

缺少专属、一律走兜底卡片的：

- `web`：`Web` 渲染器（fetch/search 两套布局）已经写好，但 `renderer_for` 匹配的是 `fetch`/`search`/`browser`，`"web"` 落兜底。接法：匹配 `"web"`，分节按 args 里的 `op`。
- `context`：`query` 输出天然是表（block/tool/ok/summary），现有渲染能力没有表段。
- `memo`：只有块行的 `memo` 列被当 `details` 解析，工具本身没有视图。
- `kairos`：回执与 `result` 两段，兜底是纯文本。
- `mcp`：调用的是哪个 server、哪个 tool 只在 args 里，兜底看不出来。
- `alias`：一句确认语，可继续兜底。

不做：`final` 不渲染；`task` 随子会话面板一并定。

死匹配：`mass_edit`（`edit` 的 `type` 取值）、`fetch`/`search`/`browser`（`web` 的 `op` 取值），都不是工具名。


没有确定下一帧的条件，或者说重绘的条件，这是非常大的性能浪费。