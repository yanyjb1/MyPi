# 待发句与回显 — 目标形态

## 一趟消息的旅程

1. 用户按 Enter。输入区看最后一条喊声：闲，还是忙。
2. 闲：走 `submit`。服务端开回合，回执带 turn id。
3. 忙：这句进输入区的队列，同时写进服务端的槽。
4. loop 在阻塞点看槽。有内容就取走、开回合，门名 `mailbox`。
5. 回合收尾，历史区喊闲。输入区看队列，写新的队头。

## 忙闲

| 喊声 | 谁喊 | 何时 |
|---|---|---|
| 忙 | 输入区 | 自己发起一回合时 |
| 忙 | 历史区 | 收到任何回合的内容时，包括别人开出的回合 |
| 闲 | 历史区 | 回合收尾时 |

闲只有一个来源，前端只认最后一条喊声：最后一条是闲才算闲，任何一条忙都把它翻回忙。误判只剩一个方向——以为忙、其实闲，此时写槽会被后端当成 submit 开回合，无伤。

历史区是流的唯一接收者，所以别人开的回合（唤醒、另一端提交、压缩）由它喊忙。

`submit` 忙时照旧拒绝，不转信箱，不兜底。被拒时输入区把文本塞回输入框，并出一条 notice。

## 信箱

- 一条槽，一个会话一份。随会话存在，detach 或删除时一起消失。
- 两个动作：写、删。写是整块覆盖、必须带正文；删是清空。取走等于自动删。
- 取用前 loop 不读内容，前端可以随时改写。
- 取用即开回合，与 submit 同一条路：turn id、帧、结构回执、缓冲区归属都一样。只有门名是 `mailbox`。
- 忙时 loop 不碰槽，内容一直留着。
- 闲时写等于 submit：被取用就开回合，不是叫醒一下再把内容留在槽里。
- 一扇门：`post mailbox`。content 里有正文就是写，没有正文就是删。

## 前端队列

- 输入区持有队列，按顺序记正文与 prefix/suffix。
- 槽里永远是队头，也就是下一个要跑的。执行顺序与提交顺序一致。
- 收到 `mailbox` 回合的帧 = 队头已被吃掉：出队，写下一条队头；空了就什么也不写。
- `alt+up` 从队尾往回取一条，放到输入框最前（新取的在上，已取的在下）；取完最后一条时删槽。
- 保留区空闲那行显示队列条数。

## 回显

- 回显由输入区给：正文、prefix/suffix 都在它手上。
- 一发出就进历史区的可变区，按用户块的样子画，不等回合收尾。
- 回合结构到达时随可变区一起转交给不可变区，拿块号落户，不重排。
- 被拒或没开成回合，回显撤销，并出一条 notice。
- 队列里的消息不占回显；等它真的开成回合才出现。

## 可变区与不可变区

- 可变区：没有块号的块，正在长的那一回合，永远贴着整条历史的最底下。每帧按当前宽度重排，不参与高度预测。
- 不可变区：有号的块。号发出去不改；高度靠预测，看不见的槽不排。
- 转交：可变区拿到本回合的结构信息后，把自己那份成品交给不可变区。交出去之后可变区不再持有，不可变区也不重排。

## wake

- 文本形成时只进信箱的 wake 队列，不写库。
- 被取信时才写块：一条 wake 自己成立一个块，几条就是几个块，不合并。
- 取信有两个时机：回合收尾后的阻塞点（开一个新回合，Wake 是头一块），和回合进行中的 drain（插进当前回合中间）。
- 组请求时，连续的 wake 块并成一条 user role 消息给模型。块本身不并。
- 上流时块的分界原样保留：一条 wake 一个 `stream` 帧，`content.kind = "wake"`，正文是这一条自己的文本。前端因此能一条一条画提醒，也看得出是哪个任务完成了。
- 前端收到就画成一条提醒，进可变区，随回合结构一起转交。
- 提醒的样式待设计。

## 门表变化

| kind | name | payload | 结果 |
|---|---|---|---|
| `post` | `mailbox` | `session`，有正文时带 `text`/`prefix`/`suffix` | 写槽；没有正文就是清槽 |

`subscribe / submit` 不变，只是不再承担排队。

## 流上的 piece 清单

`text`、`thinking`、`arguments`、`boundary`、`tool_start`、`tool_progress`、`tool_end`，加一项 `wake`。

## 文件布局

### 服务端

| 文件 | 装什么 |
|---|---|
| `loop/session/mailbox.rs` | 槽与 wake 队列：写、删、取。 |
| `loop/session/book.rs` | 唯一开门路径：`deliver`、写槽、清槽、调度、取信。 |
| `loop/turn/cycle.rs` | 回合中的 drain：只取 wake，合成一段。 |
| `loop/pen/turn.rs` | 回合打开时写 Wake 块。 |
| `request/live.rs` | 事件通道的形状，加 wake 一支。 |
| `api/post/mailbox.rs`（新） | 信箱门：解 content，派到写或删。 |
| `api/route.rs`、`api/post/mod.rs` | 门表登记。 |
| `api/relay.rs` | 回合记账：信箱回合与 wake 回合的 turn 与门名。 |
| `api/subscribe/submit.rs` | 忙时拒绝，开门。 |

### 前端

| 文件 | 装什么 |
|---|---|
| `zone/main/input/queue.rs`（新） | 队列：顺序、内容、队头同步、取回。 |
| `zone/main/input/mod.rs` | 提交路径：闲走 submit，忙入队并写槽。 |
| `zone/main/input/semantics.rs` | `Alt+Up` 这一条意图。 |
| `zone/main/reserved/queue.rs`（新） | 保留区那行提示与条数。 |
| `zone/main/history/live.rs` | 可变区：助手、工具、wake、回显。 |
| `zone/main/history/transfer.rs`（新） | 转交：结构到了之后，把可变区的成品交给不可变区。 |
| `zone/main/history/feed.rs` | 帧分派；wake 与回显的入口。 |
| `zone/contract/shout.rs` | 新增的名字。 |

### 职责

服务端管槽、wake 队列和记账；api 管门与回合标号。前端里输入区管队列和「这一句走哪条路」，保留区管条数那一行，历史区管可变区、转交和画。

### 文档

`api/README.md`、`tui/README.md`、`zone/main/README.md`、`zone/main/input/README.md`、`zone/main/reserved/README.md`、`zone/main/history/README.md` 跟着改。
