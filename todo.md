# TODO

整个项目剩下的活。收尾期，按能开工的先后排。

施工完毕后，务必更新本文档。

##### memo

来自用户，没有用户明确允许，不允许删除。

/memo命令根本没做

main区：
1. edit/write这些的渲染简直就是灾难，diff完全看不了。这个留着大后期一起抄omp作业吧，现在的拉完了。
2. 还需要检查其他部件的这个问题。

工具卡的太抽象了，我不行了。。。这个渲染，我必须给每个工具都，赶紧抄一下专有的渲染样式以及动画样式了。

记录一下工具卡为什么抽象，这里点名批评bash，捏吗的，模型都看到结果了，bash的渲染还没完成，一定要你妈的下一次http还是回合结束了才正常渲染，错误，大错误！！！😡



/new的命令，切换后无法使用/profile，疑似判断有问题，输入区持有哪些东西需要重点整治一下。。。太特么难绷了。以及其他残缺的、无法正常运行的命令。😡

可能还有一些问题，🤔总之，可以在解决渲染样式的时候，集中抄一下omp的作业。

服务端CPU异常与退出问题（另开会话处理，调研结论2026-11-08）：
1. CDP泵线程轮询：`src/web/browser/devtools/cdp.rs:22` 每条连接 read timeout 50ms（20Hz空转唤醒），连接随tab存活；附着型浏览器（`Browser::attach`，`src/web/browser/process/mod.rs:44`）被 `idle::sweep` 永不关闭（`idle.rs:27` 直接return），泵跟着活到进程死。修法：去掉50ms轮询（阻塞读+Drop关socket唤醒），或退一步拉到500ms。
2. 会话只book不unbook：`ledger.book`（`book.rs:149`、`ensure_booked`，api.rs:173）之后唯一unbook路径是删会话（`post/delete.rs:50`）。watch/mailbox/switch/cwd只增不减，会话句柄+每会话sched/jobs线程对无界累积。方案：relay维护每会话watcher计数，归零即unbook（busy时interrupt，`handle.gone`机制现成）；`ensure_booked`删除，book只留`post/init`与watcher出现两处。
3. "前端退出服务器不死"原因未定案：退出条件只看relay连接数（`api.rs:164`），需复现时`top -H`看线程名+`ss`看连接抓现场。
4. 会话存活判据统一为"有没有连接在看"，涉及api/relay/loop整体调度，故整体另开。

Anthropic 的 usage 与其余三家不同义（2026-11-08，用户暂时不用 Anthropic，挂起）：
`protocol/anthropic/messages.rs` 的 `usage_from` 取 `prompt = input_tokens`，而 Anthropic 的 `input_tokens` 不含 `cache_read_input_tokens` / `cache_creation_input_tokens`。其余三家（OpenAI completion、DeepSeek、Responses）的 `prompt_tokens` / `input_tokens` 都已包含缓存部分，于是 `Usage::prompt` 这一个字段在 Anthropic 上语义不同、数值偏小。连带三处：`api/call/usage.rs` 的 `quote()` 里 `uncached = prompt − cached` 在 Anthropic 上 saturating 到 0（输入费一分不收）；`cache_hit_rate = cached / (prompt + completion)` 分母偏小（命中率虚高）；`estimate::context` 把 `stamp.prompt` 直接当上下文（偏小）。修法：解码时 `prompt = input_tokens + cache_read + cache_creation`，`quote()` 的 `uncached` 同时减掉 `written`。存量的 `usage_prompt` 列不回算。

## 一、历史区

- 滚动到未取内容的块时，历史区仍用 `…` 占位，没有按需取块；树区已经按屏取。

### 待设计

- wake 提醒的样式。现在一条 wake 一个块、一条一个 `stream` 帧，画成一行 notice（`wire/live.rs` 的 Wake 分支），落地后却画成用户块。
- 每类工具的流式动画。历史区只提供位置，运行中的卡片还是完成态。

### 其余

- 转交的兜底：尾块数与结构区间数对不上时，清空可变区按收到的行重铺，会闪一帧。

## 二、main 区

- 内部各组件 shout→react 的行为没有核对过。
- 写门被拒这件事仍有两处静默：另一条连接（不是本轮的写入者）发写门、`/compact` 的拒绝（走回合的 broken 提示，前缀不是给人看的）。`/profile` 已在输入区本机拒绝。
- 状态栏的三次启动提问是 `gitScanInterval`、`fx.currency`、`api.defaultModel`；没有 Setup，也没有重建。会话还没名字时 session pill 显示 `MyPi`；cost 恒显示，未知即本币 `0.00`；git 扫描没落地前显示 `@ ?`。

## 三、已知代价（记录，暂不修）

- 卡片按当前宽度自己折行，宽度一变该块的折行缓存整块作废；可视层的窗口锚点是块内行偏移，宽度变化后按新布局钳制。

- 可见槽的每一帧都复制一次行。
- `run` 每帧无条件重绘，`Term::poll` 固定 16ms。
- 只有当 worker 起不来、退回直连时，取消才靠 1 秒轮询；那条读线程要等上游关连接才退出。
- 补全每开一次手问一次 `call pools`，一次本地往返。
- worker 与替代者之间靠一轮 `kill` 子进程切换，不走内核的进程组回收。
- 每次 attach 把启动目录写回会话行（启动目录为准）；目录相同则不写。那个会话正被别的写入者跑着时，这次写被静默拒绝，目录保持原样。
- 压缩之后、下一次请求之前，上下文条是估的（摘要 + 按截断点权重推算的尾巴 + 档案地板），带 `~`；下次请求落地即换成实测值。
- git 扫描在帧里同步跑：一次 `git status` + 一次 `git branch`，每间隔一次几十毫秒。
- 临时 `cd` 只活在前端与内存：重启后以库里会话行的 cwd 为准。

### web

- 每次 web 调用新建一条 CDP WebSocket，一条连接配一条泵线程，没有复用。
- 浏览器存活判断是端口探测，死端口上每次最多 500ms，挂在 30 秒定时器线程上。
- attach 路径对同一个端口探测两次（判断 `DevToolsActivePort` 是否陈旧、校验用户端口），多一次 TCP connect。
- 换浏览器实例时（闲置关闭后重启、崩溃重启）别的前端在飞的 web 调用会拿到连接重置，只能由模型重试。
- 验证轮询最长 5 分钟，期间占着一个 permit 和该会话的搜索锁。
- 引擎名单在工具 schema、错误消息、静态表三处各写一遍，加一个引擎要改三处。
- 服务端在启动浏览器的那十几秒里收到退出信号时，信号线程要等 `STATE` 锁释放后才开始关树。

## 四、树区

- 多 fork 投影：几何支持，数据源还没有。

## 五、工具层

- 浏览器归服务端管了：有头、懒启动、闲置 10 分钟关整棵进程组、服务端退出与 INT/TERM/HUP 先关树，Linux 上还有 pdeathsig 兜 SIGKILL。剩下的跨平台口子在 Windows 的 Job Object 与 macOS 的守护补齐，见 `src/web/README.md` 的 Unresolved。
- 有头浏览器需要显示环境；无显示的服务器上 web 工具整体不可用，这是设计取舍。
- 三家解析器按实采页面调过；引擎改版后“解析为空”要当选择器问题查，不是门页。
- 百度广告卡混在自然结果里（卡片自带真实目的地，抓到的页面没有稳定标记）。
