# 本轮记录

日期：2026-10-21

用途：把这一轮关于服务端、API、配置、profile、事件、memo、alias 的讨论落盘。下一次只在这个文件上继续，不重复问已经定过的东西。

标记：

- **定** = 用户已经拍板。
- **候选** = 我根据讨论推出来的，还没拍板。
- **否** = 讨论过但被否掉，别再提。
- **留给工具设计** = 等 memo / alias 专题再定。

---

## 1. 总原则

### 1.1 服务端 API 与 session 门面分开

**定**：服务端 API 只提供原料和事实；session 是前端内部负责和后端打交道的部分，自己组合初始量、窗口、结构缓存、跳转和事件。

**定**：结构缓存一次返回整个结构不是问题。树、session、未来的历史区都消费它。

**定**：不能因为 session 需要某种取数形状，就让服务端 API 长成 session 的形状。两层之间是适配关系。

**定**：服务端不关心显示权、zone、事件积压。它只按 viewer 投递。

### 1.2 讨论来源

**定**：以后讨论先分设计文档和当前代码。`README` 第 2/3 节、`memo.md`、`VISION.md`、`重构待办.md` 里的设计条目是未来；`ARCHITECTURE.md` 和代码只是现状证据，不作为设计依据。

**否**：把当前代码的实现缺口当成设计项，再和设计文档混在一起。

---

## 2. 服务端 API

### 2.1 Call

**定**：Call 只留这些。

```text
Init { session, limit }
    -> Reply::Init { structure: Vec<TurnShape>, blocks: Vec<Block> }

Blocks { session, ranges: Vec<[i64; 2]> }
    -> Reply::Blocks(Vec<Block>)

Meta { session }
    -> Reply::Meta(SessionState)

Usage { session, lang }
    -> Reply::Usage { lang, groups }

Config { key }
    -> Reply::Config(ConfigValue)

Pools
    -> Reply::Pools(CandidatePools)

Sessions { under }
    -> Reply::SessionRows(Vec<SessionRow>)

Todo { session }
    -> Reply::Todo(Vec<TodoPhase>)
```

**定**：`Init` 给完整结构加最新 `limit` 个块。结构新到旧；块 id 升序。结构已经说明还有什么，所以不需要 `more`。

**定**：`Blocks` 只按区间取。闭区间；按请求顺序；区间内 id 升序；重叠不去重；越出这条会话的区间报错。

**定**：`Meta` 返回会话行和当前内存 cwd。模型 `cd` 的临时目录不落库，只能从这里读。

**定**：`Config` 只接受注册表里的键；未知键报错；返回注册表类型，不裸 JSON。

**否**：`Call::Tail`、`Call::Older`、`Call::Newer`、`Call::Turn`、`Call::Log`。

**否**：`Log` 给前端看。服务端日志只留在进程内。

### 2.2 Post

**定**：Post 保留并新增。

```text
Name { session, name }
Cwd { session, path }
Switch { session, id }
DefaultModel { id }
Profile { session, profile }
Init { cwd, model, name, profile }
Delete { session }
Interrupt { session }
Rewind { session, turn, block }
Unwatch { session }
Reload
```

**定**：所有写都归到当前 viewer。`post::apply` 收 `Viewer`，不再只收 `Post`。

**定**：`Unwatch` 不再自己带 viewer；viewer 由门带进来。

**定**：`Profile` 只改 pending；真正生效只在两个边界：新会话首条消息之前、压缩后首条消息之前。中途不换。

**定**：`Rewind` 只做 prune。`pruneOnRewind=false` 在配置加载层 panic，进不到这里。

**定**：`Reload` 重读配置。任何会话有活跃回合时返回 `Busy`。

### 2.3 Subscribe

**定**：`Submit` 属于 subscribe，不是 call/post。它只回 `Opened`；回合内容从 watch 的事件流来。

```text
Watch { viewer, session } -> Receiver<Event>
Submit { session, prefix, text, suffix } -> Opened
Compact { session, retention, focus } -> Opened
```

**定**：`Opened::Started { turn: Option<i64> }`。提交的回合号已知；压缩的回合号在 `Ended` 里给。

**否**：`Submit` 一次性返回块。持续输出只走 watch。

### 2.4 Watch

**定**：watch 是 API 订阅，不是 App Event。

**定**：watch 只给增量。初始量走 `Call::Init` 和 `Call::Meta`。

**定**：watch 带两类东西：数据库里已经有的元数据变化、聊天内容增量。

**定**：元数据只推数据库里有的：name、cwd、model、profile、usage。模型 `cd` 的临时目录不推；上下文占用百分比这种算出来的不推，前端自己算。

**定**：旧内容和旧通知都不补。旧块用 `Call::Blocks` 按区间拿。

**定**：新通知包含在聊天内容增量里；旧通知不补。

### 2.5 Event

**定**：服务端事件是前端系统事件的一种；它不是前端 Event 的复制。

```text
Started { session, turn: Option<i64>, kind: StartKind }
Live { session, turn, blocks: Vec<NewBlock>, done }
Blocks { session, turn, blocks: Vec<Block> }
ToolOutput { session, call_id, text }
Ended { session, turn: Option<i64>, stop_reason, usage: Option<RoundUsage> }
Meta { session, meta: SessionMeta }
Rewound { session, turn, block }
Deleted { session }
Notice { level, content }
```

**定**：`Meta` 只推数据库里已经有的会话行字段；模型 `cd` 不推。

**定**：`Ended` 带本回合 usage。详细账单仍走 `Call::Usage`。

**定**：`Notice` 是服务端旁白。`content` 里需要 UI 文案时写 `{{placeholder:key}}`；前端渲染时替换。

**定**：服务端事件先到 session；session 更新 view，再发信号；App 把信号转成前端 Event；zone 按声明消费。

**定**：前端 Event 仍然只是信号；数据在 view 和 Ctx 里。

### 2.6 Notify

**定**：`api::notify(viewer, level, content)` 是服务端内部调用口，不是前端发来的 call/post。

**定**：只锁 viewer，不带 scope。服务端按 watch/attach 状态决定收件人；没有 watch 时进 viewer 的旁白队列，下一次 watch 时补发。

**定**：旁白只投一次。session 收下放进 view，App 再转 Event。

**定**：UI 自己的旁白不走 Notify，但可以复用同一个 Notice 形状，直接塞进 view。

**否**：Notify 带作用域字段。作用域已经由收件人和 zone 声明解决。

**否**：Notify 带名字和参数。content 里已经包含；需要 UI 文案用 placeholder token。

### 2.7 Viewer

**定**：Viewer 是前端生成的 UUID 字符串；服务端只当不透明值。

**定**：服务端不生成 Viewer。

**留给工具设计**：重连后算不算同一个 Viewer。当前按前端生成新值处理。

### 2.8 写入权

**定**：服务端内存维护每条会话的 owner 和 attach 顺序。

**定**：第一个写者成为 owner；后来者默认 watcher。

**定**：idle 时 watcher 写成功，owner 转移，原 owner 变 watcher。

**定**：回合中 watcher 写返回 `Busy`。

**定**：owner detach 或断线，按 attach 顺序转给下一个 watcher；没有 watcher 就空着。

**定**：owner 不进数据库，不进 watch。前端不需要实时知道；写失败时由服务端裁决。

**定**：同时写先到先得，不设计无意义竞态。

**定**：idle = 没有活跃回合。

### 2.9 前端 Event 的归属

**定**：Event 是 App 解析、App 下发。

**定**：zone 像声明键位一样声明自己消费哪些事件类。

**定**：App 只把事件投给当前显示区；一个事件只投一次，投完就删。

**定**：handover 时，App 拿新 zone 的声明去过滤积压；它消费的，按顺序发；不消费的，留在积压里，等下一个声明消费的 zone。

**定**：没人消费的事实事件一直留着，直到下一个消费者；进程退出时整队清掉。不超时、不丢弃。

**定**：状态值只读 Ctx，不进事件队列。

**否**：用“状态型 / 事实型”作为事件分类轴。事件只按来源分系统事件和自定义事件；状态不是事件。

---

## 3. 存储与 profile

### 3.1 sessions

**定**：`sessions` 字段：

```text
id, name, started_at, cwd, model_provider, model, profile
```

**定**：删除 `todo_on`。

**定**：`model` 不做历史；当前模型就是会话行上的值。

**定**：`cwd` 继续独立 `cwd_history`，不并进 profile 历史。

### 3.2 request_meta

**定**：`request_meta` 增加 `profile`，存档案名，作为请求归档的参考。

**定**：旧会话恢复以 request_meta 为准；profile 只是参考，不单独做快照。

### 3.3 profile

**定**：profile 只在新会话首条消息之前、压缩后首条消息之前切换。中途不换。

**定**：`sessions.profile` 存当前段的档案名。

**定**：压缩块 memo 记本段的 profile 指纹。

**定**：不建 profile 历史表，不建 profile 快照表。

**定**：同名 profile 改内容，指纹变；旧会话引用旧 request_meta，新会话引用新内容。

**否**：把 model 和 cwd 塞进 profile 历史或 meta_history。

**否**：profile 内容快照表。

### 3.4 todo

**定**：todo 由前端自己持有。初始量从历史来；之后跟增量块。

**定**：服务端不插 true/false，不维护“在用 / 用完”。

**定**：删除 `todo_on`；`Call::Todo` 直接重放会话的 todo 调用。

**否**：前端算出“在用 / 用完”再 post 给服务端。

### 3.5 fork

**定**：fork 设计、预留 schema、相关函数全部保留，不删。

**定**：配置加载层直接 panic 拒绝 `pruneOnRewind=false`。服务端整体是 fork 的把门人。

**定**：panic 旁边留 TODO，说明 fork 未施工；fork 做完再拔掉这道 panic。

**定**：当前只有 true 能启动。

**否**：删掉 fork 设计。

**否**：在各功能里零散兜底 false。

---

## 4. 配置

### 4.1 根

**定**：项目根优先，全局根兜底，最后默认值。按 key 取，不是整文件覆盖。

**定**：`models.yml` 必须存在，缺了直接 panic。其他文件缺、键缺，全部默认兜底。

**定**：`MYPI_CONFIG_DIR` 设置时只认那个根。

### 4.2 解析

**定**：键名 area.path.name；只用 ASCII 点号；段用 camelCase。

**定**：配置文件里未知键忽略；API 查询未知键报错。

**定**：YAML 语法坏了直接不通过，不启动。

**定**：值类型不对当缺失，继续往兜底走。

**定**：同一个键写两次：优先非默认值；多个非默认取最下面；全是默认也取最下面。

### 4.3 bootstrap

**定**：`config.yaml` 不存在时，在实际选中的 `models.yml` 旁边生成默认文件。

**定**：生成内容带区域、层级、注释；父级在前，子级在后。

**定**：已有 `config.yaml` 不覆盖；缺失键按默认值补进内存。

### 4.4 查询

**定**：`Call::Config` 按键取当前内存配置；前端不读文件。

**定**：未知键报错。

**定**：返回类型按注册表；具体强类型枚举还是别的表示，属于实现细节，不再问。

### 4.5 写回

**定**：写回写当前生效的那一层：项目值生效写项目根，全局值生效写全局根；只有默认值、没有来源时写全局。

**定**：没重载就不生效，这是用户自己的事；重载后生效，或者下次系统自己重读配置时生效。

### 4.6 重载

**定**：一条命令触发服务端重读配置；整体替换；失败保留旧配置。

**定**：回合跑着时不许重载。

**定**：models 缺了直接 panic；非 models 缺了走兜底。

**定**：别的前端不通知；拿到写权时服务端自动重载一次。

### 4.7 注册表

**定**：注册表是模板、查询 API、默认值的唯一来源。每条记录至少：完整键名、类型、默认值、区域、注释。

**定**：实现用声明宏，不引过程宏。具体宏怎么生成结构体，属于实现细节。

**定**：区域按 server、ui、tui 分组；区域内父级在前、子级在后。

**定**：每个键上方一行注释，只给生成出来的模板看；解析时忽略。

### 4.8 新增键

**定**：`config.ui.session.askedEachTime`，默认 32。

**定**：`config.ui.placeholder.switchBill`，允许空。

**定**：web 四个上限保留：fetchChars、browserChars、searchHits、pageBytes。

---

## 5. 服务端文档

**定**：两份文档已写：

- `server-目标形态.md`
- `server-施工注意.md`

**定**：目标形态给用户审核，重点文件组织、函数、API 形状。工程细节按当前行号写，给我自己施工用。

**定**：行号会漂移，后续按符号定位。

---

## 6. memo 工具

状态：**留给工具设计**。下面是用户原话要点和候选细化，不是已定方案。

### 6.1 用户原话要点

- 现在让 AI 写 memo，常常是笼统写进同一个文件。
- 同一个文件里分不清每一段是什么时候写的、谁先谁后。
- 想要“一次一个条目一个 entry”，每条有自己的时间。
- 这是服务端数据库结构要改的新需求。

### 6.2 候选形状

候选表 `memos`：

```text
id           INTEGER PRIMARY KEY
session_id   INTEGER NULL
turn_id      INTEGER NULL
content      TEXT NOT NULL
created_at   TEXT NOT NULL
updated_at   TEXT NULL
kind         TEXT NULL
tags         TEXT NULL
```

候选说明：

- `created_at` 用现有 stamp 形状 `YYYY-MM-DD HH:MM:SS`，和别的表一致。
- `session_id` / `turn_id` 可空；空 = 全局 memo，非空 = 挂在某条会话/回合。
- `kind` 可用来分“笔记 / 决策 / 待办 / 观察”，但现在没有需求，先不急着加。
- `tags` 用 JSON 数组或逗号分隔；没有搜索需求前可以先不加。
- 排序按 `created_at`，同一秒再按 `id`。
- 写入只追加；修改是否允许，待定。

### 6.3 工具接口候选

```text
memo add <content>
memo list [limit]
memo search <text>
memo delete <id>
```

候选：

- `add` 必填 content，可选 session/turn 由服务端从上下文带入，不让模型手写。
- `list` 默认按时间倒序，回最近 N 条。
- `search` 先做 `LIKE`，不做 FTS；没有需求就不引索引。
- `delete` 是否允许，待定；如果不能删，就只追加。
- 模型看到的时间由服务端填，不接受模型自带时间。

### 6.4 归属与权限

需要定：

- memo 是全局、按项目、按会话，还是三者都有。
- 跨会话可见性：A 会话写的 memo，B 会话能不能读。
- 写入权：是否要求 owner；watcher 能不能写。
- 删除权：用户能不能删 AI 写的 memo。

候选倾向：

- 默认按项目/全局，不按会话隔离；但保留 `session_id` 做来源。
- 写入要求当前会话 owner；读不限。
- 删除只允许用户/前端，不给模型删除权。

### 6.5 前端与 API

需要定：

- 是否要 `Call::Memos` 读口，还是只让工具读写。
- UI 是否要一个 memo 面板；如果要，事件怎么推。
- 新 memo 是否走 watch 的元数据/通知，还是只落库。
- 是否要在 `Call::Init` 里带最近的 memo。

候选：

- 先只做工具 + 数据库，不做 API/UI；等前端需要再开读口。
- 如果要推，新增 `Event::Memo`，不要塞进 Notice。

### 6.6 迁移

需要定：

- 旧 memo 文件是否导入。
- 导入时时间怎么定：文件 mtime、导入时间，还是每段手动时间。
- 旧文件是否保留。

候选：

- 不自动导入；给一条一次性命令，按文件 mtime 或导入时间写入，避免猜。

---

## 7. alias 工具

状态：**留给工具设计**。下面是用户原话要点和候选细化，不是已定方案。

### 7.1 用户原话要点

- 不做复杂功能，很多复杂功能可以通过快捷方式实现。
- 例如自动找出某文件夹里最大的文件，本来一个脚本就够。
- 但每次重复输入一大串前置命令很浪费 token。
- alias 允许 AI 自定义一串命令，例如 `clean -> rm ./tmp*.txt`。
- 下次调用只需要 `clean`。
- 相当于给 AI 一个允许它自己写的 bashrc，但只是别名。

### 7.2 候选形状

候选表 `aliases`：

```text
id           INTEGER PRIMARY KEY
name         TEXT NOT NULL
command      TEXT NOT NULL
description  TEXT NULL
scope        TEXT NOT NULL
created_at   TEXT NOT NULL
updated_at   TEXT NULL
```

候选说明：

- `scope` 候选：`global` / `project` / `session`。
- `name` 需要唯一约束；同一 scope 下不能重名。
- `command` 是展开后的 shell 命令模板。
- `description` 给模型看，帮助它决定用哪个 alias。
- 时间用现有 stamp。

### 7.3 展开语义

需要定：

- alias 只展开第一个 token，还是整条命令匹配。
- 带参数时怎么拼：`clean foo` -> `rm ./tmp*.txt foo`，还是 `$@` 占位。
- 是否允许管道、重定向、变量。
- 是否允许 alias 调 alias；允许的话最大深度、循环检测。
- 展开后是否再过 bash guard；应该过，不能绕过。
- 引号、转义、通配符怎么处理。
- alias 名和系统命令同名时谁优先；候选：alias 优先，但 `command clean` 可绕过。
- alias 名和 shell 内建/保留字冲突时怎么办。

候选：

- 先只支持“第一个 token 精确匹配 + 其余参数原样追加”。
- 不支持递归；发现 alias 名再次出现就报错。
- 展开后整条命令再过现有 bash guard。
- 不做 shell 级 alias，只在 bash 工具执行前做文本展开。

### 7.4 工具接口候选

```text
alias add <name> <command> [description]
alias list
alias remove <name>
alias show <name>
```

候选：

- `add` 写库；同名同 scope 报错或覆盖，待定。
- `list` 回当前 scope 可见的 alias。
- `remove` 是否允许模型删，待定。
- `show` 给模型看展开后的命令。

### 7.5 归属与权限

需要定：

- alias 是全局、项目、会话，还是三层都有。
- 跨会话可见性。
- 写入权是否要求 owner。
- 删除权。
- alias 是否跟 profile 绑定；换 profile 是否换 alias 集。

候选：

- 先按项目/全局，不按会话；保留 `scope` 字段。
- 写入要求 owner；读不限。
- 模型可以 add/list/show；remove 只给用户/前端。

### 7.6 与 bash 工具的关系

需要定：

- alias 展开发生在 bash 工具的哪一步：参数解析后、guard 前，还是 shell 启动前。
- alias 输出是否进 `ToolOutput` / memo。
- 失败时错误信息是否指出 alias 名和展开后的命令。
- 是否要把展开后的命令写给模型看，避免它以为执行的是缩写。

候选：

- bash 工具拿到 command 后，先查 alias 表；命中就展开；展开后的命令写进结果/旁白，让模型知道实际执行了什么。
- guard 看展开后的命令，不看别名。
- 展开失败（未知 alias、循环、空命令）直接报错，不落块。

### 7.7 与 memo 的关系

- memo 记“为什么定这个 alias”，alias 记“命令本身”。
- 不强制联动；alias 可以有 description，不必依赖 memo。

---

## 8. 两个工具对现有设计的影响

### 8.1 数据库

候选新增表：`memos`、`aliases`。都不是现有表的列，独立表更干净。

**需要定**：是否都要 `session_id` / `turn_id` 来源；是否都要 `scope`；是否都要 `updated_at`。

### 8.2 API

候选新增 call：

```text
Memos { session?, limit? }
Aliases { scope? }
```

候选新增 post：无；工具自己写库。

候选新增 event：`Memo`、`Alias`，只有前端要显示时才加。

### 8.3 工具注册

两个工具都进 `agent/tools/registry.rs` 的 `TOOLS`，每个工具一个目录：

```text
agent/tools/memo/
agent/tools/alias/
```

工具遵守现有规则：参数表是唯一声明；输出走 `ToolOutput`；不直接读库，能通过 `Env` 拿到的才拿；不能绕过 bash guard。

### 8.4 配置

是否需要配置键：

- `config.server.agent.tools.memo.*`：默认条数、是否允许删除。
- `config.server.agent.tools.alias.*`：最大展开深度、是否允许递归、默认 scope。

这些还没定。

---

## 9. 被否掉的方案

- 否：服务端 API 长成 session 的 Init/Round/Range 形状。
- 否：结构缓存一次返回整个结构是问题。
- 否：watch 一次性给历史；watch 只给增量。
- 否：watch 补旧通知。
- 否：Call::Log 给前端看日志。
- 否：Call::Tail / Older / Newer / Turn。
- 否：Submit 一次性返回块。
- 否：todo 由服务端维护开关，前端 post true/false。
- 否：todo_on 作为读口门。
- 否：profile 历史表、profile 快照表、meta_history。
- 否：model 和 cwd 塞进 profile 历史。
- 否：cwd 并进 profile 历史；cwd_history 独立。
- 否：删除 fork 设计；fork 保留，配置闸门 panic。
- 否：Notify 带 scope、名字、参数。
- 否：用状态型/事实型给事件分类。
- 否：事件消费后还广播给多个 zone。
- 否：前端读本机配置文件。
- 否：配置项目根/全局根整文件覆盖；按 key 合并。
- 否：models.yml 缺失时兜底；必须 panic。
- 否：配置语法错误时猜；直接不通过。
- 否：重载失败设计；models 缺失 panic，其他兜底。
- 否：别的前端配置重载通知；拿到写权时自己重载。

---

## 10. 现在只需要继续的事

**定**：服务端两份文档已落盘；接下来集中设计 memo 和 alias。

memo 先定：条目字段、归属、读写权限、是否要 API/UI、旧文件迁移。

alias 先定：作用域、展开语义、参数拼接、递归/循环、guard 顺序、增删权限、和 bash 工具的关系。

两个工具都先出目标形态，再出工程细节；不要回头重问服务端 API、watch、event、config、profile、todo、fork。

---

## 11. oh-my-pi 的唤醒机制（研究记录）

源码：`/tmp/oh-my-pi`，commit `35352fc`。以下只描述它的现状，不是 MyPi 的决定。

### 11.1 结论

它没有把异步结果塞回原来那次工具调用的 `tool_result`。原来的工具调用只返回一句“已转到后台”，真正的结果是一条**新的合成消息**，这条消息会开启下一轮模型调用。

这条合成消息不是 `user` 角色。它走 `role: "custom"`、`customType: "async-result"`，进入模型请求前被转成 `developer` 角色（`packages/agent/src/compaction/messages.ts:217` 附近）。所以更准确的说法是：**系统以开发者身份插入一条新消息，并让它开启一个回合**，不是伪装成用户原话。

### 11.2 完整链路

1. 工具注册后台作业。
   `bash` 的 `async: true`、自动后台、`task` 的子代理、`eval` 的后台单元，都注册到 `AsyncJobManager`。作业带 `ownerId`（哪个 agent 的）。完成或失败时调用 `#enqueueDelivery`（`packages/coding-agent/src/async/job-manager.ts:429`、`:438`）。

2. 作业管理器按 owner 找投递口。
   `registerDeliverySink(ownerId, sink)`（`job-manager.ts:695`）。每个 `AgentSession` 在构造时为自己的 agent id 注册 sink（`session/agent-session.ts:2348`）。没有 sink 就 dead-letter，不会串到别的会话。

3. sink 把结果变成 `async-result` 条目。
   `#deliverAsyncJobResult`（`agent-session.ts:3301`）格式化结果，然后 `yieldQueue.enqueueWithReceipt("async-result", entry)`（`:3314`）。

4. `YieldQueue` 决定什么时候送。
   - 会话正在流式输出：不立刻开新回合；条目留在队列里，等 agent loop 在工具边界通过 `drainLazy()` 取走，作为不打断当前回合的 aside 注入（`session/yield-queue.ts:180` 附近；`agent-session.ts:2199` 把它接进 `setAsideMessageProvider`）。
   - 会话空闲：`YieldQueue` 安排 idle flush（`yield-queue.ts:107` 附近），`injectIdle` 直接调用 `agent.prompt(messages)`（`agent-session.ts:2132` 附近）。`prompt` 就是开新回合的入口。

5. 结果消息长什么样。
   `buildAsyncResultBatchMessage`（`session/async-job-delivery.ts`）生成：
   `<system-notice>Background job bg_N has completed. Resume your work using the result below. ...</system-notice>`
   多个作业完成时合成一条，避免每个结果开一个回合。

6. 停止时怎么知道“还没完”。
   `#hasPendingAsyncWake`（`agent-session.ts:3225`）同时看三件事：
   - 本 agent 还有未抑制的 running job；
   - 管理器里还有排队/在飞的 delivery；
   - yield queue 里还有可投递的 `async-result`。
   只要有一项，停止就被当作“调度暂停”，`agent_end` 带 `willContinue: true, awaitingAsyncWork: true`（`agent-session.ts:4909` 附近），`session_stop`、空闲压缩、todo 提醒都往后排。

7. `wait` 只是前台取结果的另一条路。
   `wait` 等第一个完成的作业或 peer 消息；它取走的结果会标记为已消费，不再自动投递。模型不调用 `wait` 时，结果也会自动送到。`proc://` 可以查看作业而不消费；`write proc://<id>/kill` 取消。

### 11.3 它刻意处理的边界

- owner 隔离：作业只投给注册了 sink 的 owner；owner 不在就丢弃，不跨会话。
- 会话切换：`#asyncDeliveryEpoch` 递增，旧的 `async-result` 从 yield queue 清掉，防止上一个会话的结果唤醒新会话。
- 作业状态：running / completed / failed / cancelled；前台作业被抑制投递，转后台时才解除；`consumeJobResults` 防止重复。
- 重试：投递失败指数退避；结果行保留一段时间；消费后短时间清掉。
- 没有定时器：oh-my-pi 的这套唤醒是**完成驱动**，不是 cron。它没有跨进程的定时任务；定时提醒要由另一个唤醒源产生同样的消息。

### 11.4 对 MyPi 的直接翻译

MyPi 现在只有 `Submit` 能开回合，`Running::claim` 保证一条会话同时只有一个回合。要支持异步，必须增加一个**内部开回合的入口**，但不要把它做成第二个 `Submit` 用户入口。

建议的抽象：

```text
Wake {
    session,
    source: Job { id, name } | Timer { id } | ...,
    text,
    ts,
}
```

`Wake` 不是工具结果，是一条待插入的消息。它落地后开一个回合；模型看到的是“系统唤醒”，不是用户输入。

关键决定：

- 唤醒消息用什么角色：MyPi 的 `WireMessage` 目前只有 system/user/assistant/tool。可以加 `developer`，也可以映射成 `user` 加固定前缀。前者语义正确，后者兼容性最好。我倾向后者：块类型叫 `wake`，线上仍是 `user`，前缀写死为 `[system wake: ...]`，前端按块类型渲染成旁白。
- 会话正在跑时：先排队，等当前回合结束再开唤醒回合。oh-my-pi 会在工具边界注入 aside；MyPi 的 loop 是同步的，先做“回合结束再唤醒”更稳，不打断正在跑的工具。
- 多个结果同时到：按会话合成一个 wake 块，只开一个回合，避免连环唤醒。
- 没有 watch 时：仍然可以唤醒并落库；要不要限制成“有 watch 才唤醒”是配置问题，不是机制问题。

### 11.5 异步任务的边界

- 作业、定时器都只在**当前进程**里；进程退出就没了。不写作业表，不跨进程恢复。
- 作业属于一条会话；会话删除时取消它的作业和定时器。
- 定时任务只做一次性 `at`；不做跨进程的 `every`。进程活着时可以重复，但重启后不补，这符合“不做 openclaw 那种自动醒来”。
- 异步作业的第一次工具结果只写“已启动，结果会作为唤醒消息回来”；最终输出走 wake，不走原 tool_result。
- 取消、状态、列表是同一个作业管理器的读口；不需要前端 API 也能先用工具完成。

---

## 12. memo 与 alias 的数据库候选

### 12.1 共同前提

- 当前数据库是全局的 `history.db`，不是按项目分库。所以“项目”只能作为表里的一个键。
- `SCHEMA_VERSION` 要加一；按现有规则，版本不匹配整库重建。项目未发布，不做迁移代码。
- 两张新表都要进 `prepare` 的 drop 清单，否则重建时旧表会留下。
- 工具不直接写 SQL。`Env` 里加端口（trait），由 driver 实现；这样 `agent/tools` 仍然不知道 `rusqlite`。

### 12.2 `memos`

候选：

```sql
CREATE TABLE memos (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    scope       TEXT NOT NULL,          -- 'project' | 'global'
    project     TEXT NOT NULL DEFAULT '', -- canonical project root; '' for global
    session_id  INTEGER,                -- provenance, nullable
    turn_id     INTEGER,                -- provenance, nullable
    content     TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT
);
CREATE INDEX memos_by_scope ON memos(scope, project, id);
```

候选语义：

- 默认 `scope = project`，`project` 取会话的 `Workspace.root()`（会话开始目录，不随模型 `cd` 变）。
- `global` 的 `project` 为空串。
- `session_id` / `turn_id` 只记来源，不参与默认过滤；这样一条 memo 在别的会话也能读。
- `created_at` 由服务端填，模型不能自带时间。
- 只追加，不改不删；写错了就再写一条更正。这样时间线不会乱。
- `list` 默认当前 project + global，按 id 倒序，默认 20 条。
- `search` 用 `LIKE '%query%'`，大小写不敏感；不做 FTS。
- 输出给模型是紧凑文本：`[id] created_at content`；`memo` 字段给前端一个结构化对象。

工具：

```text
memo {
  op: add | list | search,
  content?, query?, limit?
}
```

`add` 必填 content；`list` 可选 limit；`search` 必填 query。暂不给模型 `delete`。如果以后要删，走前端或单独的维护命令。

### 12.3 `aliases`

候选：

```sql
CREATE TABLE aliases (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    scope       TEXT NOT NULL,          -- 'project' | 'global'
    project     TEXT NOT NULL DEFAULT '',
    name        TEXT NOT NULL,
    command     TEXT NOT NULL,
    description TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT,
    UNIQUE(scope, project, name)
);
```

候选语义：

- 默认 `scope = project`，`project` 同上；`global` 的 `project` 为空串。
- 查找顺序：project 命中优先，然后 global；同名不会合并。
- `name` 只允许 ASCII 字母、数字、`_`、`-`，首字符是字母；拒绝空名和 shell 保留字。
- `command` 不能为空、不能含换行；长度上限例如 4096 字节。
- `description` 可选，只给模型和列表看。
- `add` 同名同 scope 报错，不静默覆盖；要改先 `remove`。这样模型不会误改。
- `remove` 允许模型删自己 scope 的 alias；`global` 只允许用户/前端删，避免一个项目删掉全局命令。
- `list` 只回名字、命令、描述，不回 id 以外的内部字段。
- `show` 回一条。

工具：

```text
alias {
  op: add | list | remove | show,
  name?, command?, description?, scope?
}
```

### 12.4 alias 展开

候选规则：

1. 只展开整条命令的**第一个 token**；`clean foo` 命中，`echo clean` 不命中。
2. 命中后替换成 `command + " " + 其余参数`；不支持 `$@` 占位。
3. 只展开一层；展开结果的第一 token 即使又是 alias 也不再展开，避免循环。
4. 展开在 `bash` 的 `#<ref>` 重写之前，guard 之前；guard 永远看展开后的命令。
5. 展开结果为空、含换行、或 alias 名非法，直接报错，不执行。
6. `command clean` 这种显式绕过不特殊处理；它本来就绕过 alias。
7. alias 只影响 `bash`，不影响 `fetch` / `search` / `browser` / 工具调用名。

### 12.5 端口

`Env` 加三个端口，工具只认 trait：

```text
MemoStore   add/list/search
AliasStore  add/list/remove/show/resolve
Jobs        start/schedule/list/status/cancel
```

`MemoStore` / `AliasStore` 由 driver 用 `Writer` + `Reader` 实现；`Jobs` 由 driver 的作业管理器实现。这样 `agent/tools` 仍然不碰数据库，也不碰 `Driver`。

### 12.6 API / UI

候选先不做：

- 不加 `Call::Memos` / `Call::Aliases`；工具够用。
- 不加 `Event::Memo` / `Event::Alias`；没有面板就不推。
- 如果以后要面板，再开读口和事件；不要现在把工具形状长成 UI 形状。

### 12.7 迁移

- 旧 memo 文件不自动导入；自动猜时间会制造假时间线。
- 需要时给一条一次性命令，按文件 mtime 或导入时间写入，并标明来源。
- alias 没有旧文件，不迁移。

---

## 13. 待拍板

只列真正影响形状的：

1. 唤醒消息的线上角色：`user` + 固定前缀，还是新增 `developer`。我倾向 `user` + 前缀。
2. memo 默认 scope：project 还是 global。我倾向 project，`global` 作为显式选项。
3. alias 默认 scope：project 还是 global。我倾向 project；`global` 只允许用户建。
4. 异步工具的面：一个 `job` 工具（start/schedule/list/status/cancel），还是 `bash` 加 `async` 参数再加一个 `schedule` 工具。我倾向一个 `job`，少一个工具面。
5. 会话正在跑时，唤醒是等回合结束，还是在工具边界注入。我倾向等回合结束，先不做 mid-turn aside。