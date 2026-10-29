# 内容说明：

## 块

指直接从数据库的blocks表解码得到的所有内容（包含落库的块种类），以json的形式呈现，包含所有字段的所有信息。

## 模型

区分为模型id（伴有服务商id）以及模型显示名，后者为前端使用字段。

# register

## `hello`

1. 输入：
   - `viewer` 前端自报的 id
2. 输出：
   - 一帧 `opened`，无内容，表示这条连接的流已经活了

## `bye`

1. 输入：前端id。
2. 输出：无。

# call

## `init`

1. 输入：
   - `session` 会话号
2. 输出：
   - `structure` 该会话的全部回合号、每个回合号对应的块的id区间
   - `blocks` 该会话最新64个块的完整内容
   - `todo` 待办初始量，optional

## `blocks`

1. 输入：
   - `session` 会话号
   - `ranges` 闭区间 id 段
2. 输出：
   - `blocks` 对应区间块内容

## `meta`

1. 输入：
   - `session` 会话号
2. 输出：
   - `meta` 会话行
   - `cwd` 当前工作目录
   - `name` 当前会话名字，optional
   - `model`以及profider，此处简写，以及对应的model name。
   - 时间。
   - 换而言之，就是从sessions表里根据id所以出来的全部字段。

## `usage`

1. 输入：
   - `session` 会话号
2. 输出：
   - 根据该会话号索引得到的该会话内所有回合turns对应的tokens，算出：
   - 输入tokens，输出tokens，缓存命中tokens，缓存写入tokens，总tokens，缓存命中率（也就是缓存命中tokens/总tokens的百分比值）
   - 并视是否存在切换模型的行为，算出每个模型的对应的上述的字段。
   - 而后，调取config，读取对应模型的价格，并完成汇率换算，将tokens原样与计算出的价格（中美credits三套同时返回，credits不参与汇率计算）

## `todo`

1. 输入：
   - `session` 会话号
2. 输出：
   - `phases` 待办阶段，由loop重算得出

## `memo`

1. 输入：
   - `session` 会话号，可缺
   - `op` list、show 或 history
   - `scope` current 或 global
   - `name` 名字，optional
2. 输出：
   - `data` 名字列表，或正文，或历史版本

## `children`

1. 输入：
   - `session` 会话号
2. 输出：
   - `rows` 它派生的子会话号，通过检查该会话下turns对应的blocks中类型为task，也就是工具调用的块中得出，根据块绑定，不要直接看session，子会话不直接归属于什么session。本项目所有的设计到会话归属的问题，全部通过与块绑定的方式解决。

## `config`

1. 输入：
   - `key` 配置键名
2. 输出：
   - `value` 配置值

## `pools`

1. 输入：无。
2. 输出：
   - `models` 模型候选
   - `profiles` 档案候选
   - 也许还有命令候选？看情况

## `sessions`

1. 输入：
   - `under` 目录，可缺
2. 输出：
   - `rows` 根据cwd以及cwd_history反查出的所有的sessionid，然后拿这个id去sessions表查所有字段

# post

只有写者能发 post；看者发回 `permission_denied`。

## `init`

1. 输入：
   - `cwd` 工作目录
   - `model` 模型id+provider，可缺
   - `name` 名字，可缺
   - `profile` 档案，可缺
2. 输出：
   - `session` 新会话号（唯一例外）

## `name`

1. 输入：
   - `session` 会话号
   - `name` 新名字
2. 输出：无。

## `cwd`

1. 输入：
   - `session` 会话号
   - `path` 新目录
2. 输出：
   - 无。

## `switch`

1. 输入：
   - `session` 会话号
   - `model` 模型id，以及provider。
2. 输出：无。

## `profile`

1. 输入：
   - `session` 会话号
   - `profile` 档案
2. 输出：无。

## `todo_on`

1. 输入：
   - `session` 会话号
   - `call_id` 触发开启的那次工具调用
2. 输出：无。

## `todo_off`

1. 输入：
   - `session` 会话号
   - `call_id` 触发关闭的那次工具调用
2. 输出：无。

## `delete`

1. 输入：
   - `session` 会话号
2. 输出：无。在忙时回忙碌应答。

## `interrupt`

1. 输入：
   - `session` 会话号
2. 输出：无。

## `rewind`

1. 输入：
   - `session` 会话号
   - `turn` 回到哪轮
   - `block` 回到哪块
2. 输出：
   - 无。

在忙时回忙碌应答。

## `default_model`

1. 输入：
   - `model` 模型id+provider
2. 输出：无。

## `reload`

1. 输入：无。
2. 输出：无。

# subscribe

## `watch`

1. 输入：
   - `session` 会话号
2. 输出：无。声明这个会话为当前会话，角色为看。换到别的会话即自动退出旧的，没有单独退订这回事。

## `submit`

1. 输入：
   - `session` 会话号
   - `prefix` 前置文本，可缺
   - `text` 用户文本
   - `suffix` 后缀文本，可缺
2. 输出：
   - `turn` 这一轮的轮号
   - 同时夺取这个会话的写者身份

在忙时回忙碌应答。

## `compact`

1. 输入：
   - `session` 会话号
   - `retention` 保留多少尾部：默认、token 数、块数三选一
   - `focus` 摘要侧重，可缺
2. 输出：
   - `turn` 这一轮的轮号
   - 同时夺取这个会话的写者身份

在忙时回忙碌应答。

# 流

hello 之后，每条连接就天然持有这条会话的流；发什么由 api 决定，前端不需要预先约定帧的种类。

## 事件

流里只有三类事件，外加一条收尾：

1. `post`：会话数据被改动的记录，带发起者。改名、换目录、切模型、换档案、todo 开关、删除、回退、memo 写。投递时跳过发起者，其他订阅者全收。
2. `stream`：后端产出的内容，带轮号。块的实时内容、已落库的块、工具输出。所有订阅者都收。
3. `notice`：服务端自己的播报，带级别与内容。所有订阅者都收。
4. 回合结构：本回合结束时作为最后一条发出，内容是轮号与本回合各块的 id 闭区间。

## 回合缓冲

1. 归属：会话，不归连接。
2. 寿命：跟着订阅者。有订阅者才存在，最后一个离开立刻销毁；没人订阅时事件直接丢，不建不记。
3. 内容：不挑食。这个会话上出现的任何东西都进来——`post`、`stream`、`notice`，不要求带回合号，也不按回合归类。缓冲里就是自上次清理以来到达的全部事件。
4. 清空：只有一个时机。回合结束、结构条目发出之后，整段清掉。回合之间的 `post` 自然归进下一段，不需要单独判断归属。缓冲本身保留，空着等下一个事件。
5. 建立：不需要显式建立，第一条事件到达就是建立；没订阅者时不建、不记、不产。

## 转发

1. 事件按会话号落进对应缓冲，再查连接表：当前会话对得上的连接才收。
2. `post` 跳过发起者；`stream` 与 `notice` 全收。
3. 写者由 `submit`、`compact` 夺取，一时只有一个；被夺走、换会话、退订、断线即结束。
4. 应答与流走同一条连接出口。

