# agent 目标形态：loop

关联：BEHAVIOR.md、PRINCIPLES.md、AGENT-DECISION.md。只写 loop：它对 request 的期待、对 tools 的期待、以及它自己的机制。

## 一、布局与三方契约

### 布局

四个概念：一个会话、一个回合、一笔写、一条出线。

```text
agent/loop/
  mod.rs

  session/      一个会话：它的簿、它的活物、它的信
    mod.rs
    book.rs         簿：挂号、查找、丢弃、谁在跑
    handle.rs       把手：状态访问、当前回合、等它结束
    live.rs         活物：装配与持有
    settings.rs     本回合的配置值
    transport.rs    连接：新建、换模型、换 key
    mailbox.rs      信箱：信队与布尔
    change.rs       状态变化：model、cwd
    artifact.rs     引用取件，随活物消亡

  turn/         一个回合：从开场到收尾
    mod.rs
    cycle.rs        圈循环、阻塞点、停机判定、mode 分派
    env.rs          工具环境装配：todo 重放、context 窗口
    ask.rs          交给 request：打包端点、收回答。普通回合与压缩回合同一条，按 mode 选入口
    batch.rs        交给 tools：整批执行、事件、结果回收
    fence.rs        围栏与子代理预算

  pen/          一笔写：循环写的每一行
    mod.rs
    birth.rs        会话行、计量锚点行、链接行
    turn.rs         开场与收尾的行
    blocks.rs       request 交来的块（溢出的引用随块带来）
    effect.rs       效果 → 行
    spill.rs        溢出：artifact 行，引用随块带来

  out/          一条出线：活线
    mod.rs
    element.rs      增量、分界符、工具事件、通知
    sink.rs         发端与会话过滤
```

三条判据：存储行只出现在 `pen/`，别处不拼 `Ref`；交给另外两部分的只有 `turn/ask.rs` 与 `turn/batch.rs`；压缩、子代理、wake 不许有专属分支，它们分别是回合的 mode、一次普通挂号、一条普通消息。

### loop 是什么

调度者：开场写行、发放 Reader 与端点、跑工具批、收尾写行、检查信箱、处理状态变化。

不拼历史（还原归 request，它只在装配 Env 时向 request 的读口要一次消息）；不认识块类型、不切块、不执行工具、不解码协议。

```text
storage(Reader/Writer) ──► loop ──► { request, tools }
```

### 对 request 的期待

交出去：Reader、transport（模型、provider、base_url、协议、key、兼容表已展开）、锚点标识、本会话的工具清单（子代理是子集）、子代理才有的追加系统提示词、输出上限、取消旗标、活线发端、写块口、重试口。

request 自己做：从锚点取系统提示词（追加段拼在后面）、按交来的清单取手册、读历史、还原、组装、发送、解码、切块、经写块口写块、切工具结果；压缩时读回合视图、求计划、发摘要请求、交回计划与摘要。

交回来：停止原因、用量、调用清单（call_id、名字、参数、intent）；压缩时是计划与摘要。

必须做到：块类型只有它一份、块原样入库、工具结果整批切、缺一个调用就整体失败、失败以错误返回由 loop 收场。

不得做：派工具、开回合、碰会话状态、认识任何工具的名字。

### 对 tools 的期待

交出去（Env，每回合装配一次）：工作目录与许可区、输出预算、todo、context 窗口、artifact 取件、memo 读口、jobs、subagent、bash 超时与取消、browser 设置、白名单、别名、MCP 桥、home 与 temp。

交回来：文本、memo、溢出全文、效果（agent 自有的中性枚举：memo、alias、子代理）、是否终止回合、进度增量。

必须做到：一切外部条件从 Env 拿；不开库、不读配置文件、不碰环境变量、不持 Writer；不自己量输出；只描述改了什么、不描述行；`cd` 只改自己看到的目录，批内看到的是批开始的快照；建会话的动作只回意图。

## 二、会话

### 出生

会话行由调用方先写好，`book` 只开启运行时。会话号、`provider`、`model`、`meta` 都在行上；系统提示词与工具清单由调用方从 `meta` 或外部 profile 取好，loop 不读 profile。

- 会话号：来自 `sessions` 行的自增主键；子会话行由 loop 在结果块之后写，`sessions.anchor` 绑结果块 id。
- profile：出生前由调用方解析成系统提示词与工具清单；出生后不保留、不切换。
- 系统提示词与工具清单：普通会话由调用方解析；子代理取主代理那次解析的结果，拼追加段，工具清单取模板子集。
- 第一条消息：`book` 不携带；新会话和子会话都由 `deliver` 投第一条消息。

### 计量锚点与用量基线

锚点行 = provider:model + 系统提示词 + 工具手册，按内容合并；系统提示词与工具手册分开存。

基线 = 每个回合第一圈请求的 prompt 量，挂在锚点行上。同一个锚点下，后面任何回合的第一圈若算出来更小，就更新成更小的那个。更新发生在该回合收尾。

子代理不新建锚点：它复用主代理那条，模板与契约只作追加系统提示词，不进锚点；它只贡献用量，也参与同一条基线的取小。锚点对子代理因此只是计量口径，不是请求内容的描述——它的请求多一段追加、手册是子集。

### 状态变化

一次一个字段，值已展开。

| 字段 | 规则 |
|---|---|
| model | `config.server.agent.loop.scrooge` 为真则不许中途换（报错）；为假则换 `sessions.provider/model` 与 transport，下一圈请求生效 |
| cwd | 不限，立即改会话目录并落行 |

profile 没有这条口：它只在出生时出现。

### 生命周期

- 挂号：号不存在 = 新会话（writer 分配），顺手写会话行；号已存在 = 直接挂上（历史会话，或已建过行的子会话）。
- 新会话挂上即开首回合；历史会话只挂上、不开口，等下一次投递；子代理在任务激活时才挂上。
- Live 惰性：第一次真跑回合才装配，跑完留在槽里复用。
- 丢弃：收掉运行时、取消全部 job、清理临时文件。删会话即一例。
- 投给未挂号的会话一律报错，不查库、不复活。
- 每个回合一条线程，每个会话一条后台调度线程。

## 三、回合

### 开场

一次提交：锚点行（需要时）、会话行（需要时）、回合行、首块、列表位（压缩回合带标记）。

### 圈与停机

```text
围栏? stop? 信箱?
   ↓
交 Reader 与端点给 request → 拿回停止原因、用量、调用清单
   ↓
无调用 → 收尾
有调用 → 整批串行跑，发开始与结束，进度进活线
        → 结果交回 request 切块，经写块口落地
        → pen 在同一提交里补上 artifact 行与效果行
```

收尾一次提交：settle（停止原因加由块求和的用量），并把基线取小更新。

停机四种：模型停、用户中断、连接断、被杀。中断与杀分两个旗标。

### 阻塞点与信箱

阻塞点三个：等用户消息（回合开场）、等工具整批跑完、用户打断。打断时 loop 进入阻塞点，按正常顺序检查信箱；有内容就取第一条作为新回合的起点，空则结束当前回合。

信箱每会话一个，里面两条队列：wake 与用户插话。wake 来自工具结果，确定、不可撤销；用户插话可修改、可撤销。两条队列都空，才结束回合。阻塞点先收工具结果，再取 wake，再取用户消息。wake 由 job 回调写进信箱；空闲时的用户消息走普通投递。

### 围栏与预算

- 回合围栏：`config.server.agent.loop.maxToolCalls`，数工具轮，不是数调用；假值关闭。
- 子代理预算：非 `final` 调用累计；耗尽后名单只剩 `final`，被拒的调用仍拿到失败结论文本，`final` 不计数。

### 压缩

压缩是一个回合的 mode，不是另一套机器。

1. loop 只给参数：保留量、focus。
2. request 读回合视图、求计划、发一次无工具请求、拿回摘要，连同计划一起交回。
3. 开场与普通回合相同，首块是一条压缩标记的用户消息，列表位带压缩标记。
4. 收尾前落：摘要块、区间借用、settle。
5. 失败：照常 settle 收场，上下文不动。

压缩不触发 profile 落地，也没有“压缩后换 profile”这回事。

## 四、出线与写

### 活线

| 元素 | 内容 |
|---|---|
| 增量 | 来源（正文、思考、工具参数）加文本 |
| 分界符 | 无 |
| 工具事件 | 开始（call_id、名字、参数、intent、首块文本）、进度（call_id、文本）、结束（call_id、成功、结果、memo、耗时） |
| 通知 | 级别、文案 |

分界符不承载内容；增量每片自带来源。回合结束不发元素，storage 的中转（回合号加区间）就是结束信号。

### 溢出与引用

前门量输出、切出全文。引用由切块的人（request）本地算，随块交给 loop；loop 只写 artifact 行。

1. 全文算 FNV-1a 64，得 16 个十六进制字符。
2. 引用长度从 4 位起，按 4、6、8、10、12、14、16 递增；4 位会经常撞，自动加长是常态，不是异常。
3. 取当前长度前缀去库里查一次：没有行用它就用；有行用它但整条哈希不同就加长；整条哈希相同（同一段全文）就直接复用。
4. 这个前缀写进结果文本，也和整条哈希一起写进 artifact 行的两列。
5. 取件按引用列精确匹配，拿到它的 anchor 块 id，之后一切按块 id。

比较只用哈希，不读内容。artifact 一块一份，不按内容去重；同一段全文出现两次就存两行，引用相同，取件任取一行。取件的临时文件随 Live 消亡。

## 五、子代理与嵌套

### 子代理

子代理就是普通会话，没有专属机制。

1. `task` 回意图：主会话、主根、追加系统提示词、工具子集、预算、model、cwd；锚点由 loop 从主会话取。
2. 先写工具结果块，再写子会话行，`sessions.anchor` 绑结果块 id；会话号由这次插入生成。
3. 拿到 `sessionid` 后 `book`，再用 `deliver` 投启动消息；复用主代理的系统提示词与工具清单。
4. 结束：有 final 文件用它，否则用最后一条 assistant；都没有判失败；唤醒只报状态。

规格只在内存：进程重启，子会话不能续跑，接受。`final` 只在子会话可见，`task` 只在主会话可见，`kairos` 不给子会话。

### 嵌套

工具里可以起另一个会话的回合，因此：

- 除注册表外没有进程级可变状态，一切都挂在会话上。
- 一个回合一条线程；等子回合是普通阻塞等，等的时候不攥锁。
- 会话槽的锁只在取放活物的瞬间持有，跑工具时必须已放开。
- `deliver` 谁的线程都能调；同一会话同时只有一个回合。
- 围栏、预算、信箱、活线各归各会话，全程序没有“当前会话”。
- 嵌套深度不设限，能不能再往下开由名单决定。
- 后台任务池按会话算名额，满了 `task` 当场失败，不排队。

## 六、公开面与跨域

### 公开面

```text
Loop::new(writer, settings, ask, batch, out) -> Loop
book(Birth) -> Result<SessionId>
deliver(session, Message) -> Result<Option<TurnId>>
interrupt(session)
drop(session)
change(session, Change) -> Result<()>
mailbox(session) -> Result<Arc<Mailbox>>

Birth = { session, provider, model, system, tools, meta, cwd }
        调用方先写 sessions 行；book 只开启，不写库
Message = Submit { prefix, text, suffix } | Compact { retention, focus }
Change  = Model { provider, model } | Cwd(path)
Element = Increment { part, text } | Boundary | Tool(..) | Notice { level, content }
```

其余全部私有：回合、圈、Live、Env、历史、压缩、围栏、artifact、子代理。

### 跨域要求

| 需要 | 为什么 |
|---|---|
| storage：`sessions` 加 `anchor` / `meta` | 子会话链接；系统提示词与 tools 的来源 |
| storage：`turns` 去掉档案指针 | 档案是会话级，不是回合级 |
| storage：`request_meta` 只留 provider/model/system/tools/baseline | 锚点身份与上下文预估 |
| storage：artifact 加 `hash` / `reference`，按引用列取 | 溢出引用 |
| storage：`Row::Borrow` 区间借用 | 压缩保留区 |
| storage：删除 `subsessions` 表 | 链接只在 `sessions.anchor` |
| config：提供 provider 出口（base_url、api、auth_header） | 构造 transport |
| config：新增 `config.server.agent.loop.scrooge` | 模型中途切换开关，真＝不许 |
| config：删除 `config.server.agent.reloadOnCompaction` | 没有读取方，清单出生即冻结 |
| agent 自读 `mcp.json` | 配置里已无 mcp 段 |

request、tools、config、api 的完整形状见 `loop-域外要求.md`。