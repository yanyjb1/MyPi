# Subagent 设计

日期：2026-10-22

状态：核心设计收口。

---

## 1. 定位

子代理是一个普通会话，不是第二套循环。

它有自己的一条 `sessions` 行、自己的 `Live`、自己的回合和工具调用。agent loop、工具执行、存储、Relay 全部按 session id 复用。

子代理只干活，不接受用户插话。用户能看、能打断、能删，但不能像主会话一样 `Submit`。

禁止递归：子代理永远不能再派子代理。

---

## 2. subsessions 表

不给 `sessions` 加 `parent_id`，也不加 `kind`。单独一张链接表：

```sql
CREATE TABLE subsessions (
    subsession_id INTEGER PRIMARY KEY,  -- sessions.id
    anchor        INTEGER NOT NULL      -- task 的 toolresult 块 id
);

CREATE UNIQUE INDEX subsessions_by_anchor ON subsessions(anchor);
```

只存这条链接：

- 不存 template。
- 不存变量。
- 不存 created_at。
- 不存 parent_id。
- 不单独建 request_meta。

时间看 `sessions.started_at`。实际发出去的 system 和 tools 由普通请求归档记录；那不是子会话模板快照。

`Call::Sessions` 用 `id NOT IN (SELECT subsession_id FROM subsessions)` 过滤。`Subscribe::Submit` 遇到子会话直接拒绝。

`Call::Children { session }` 先拿当前会话块列表里的块 id，再和 `subsessions.anchor` 对上，列出子会话。不靠父子字段。

---

## 3. task 工具

```text
task {
  template: "reviewer" | "/abs/path/to/reviewer.md",
  vars: { "target1": "...", "target2": "...", "APPENDED": "..." },
  cwd?: "..."
}
```

默认走 `kairos now`：派出去就返回，不等子代理跑完。

回执带：

```text
subsession_id, anchor, task_id, final_path
```

完成时 `kairos` 发 wake，主代理自己读 final 文件。

---

## 4. 模板

模板先按名字找：

1. 项目根的 `.mypi/agents/<name>.md`
2. 全局配置目录的 `agents/<name>.md`
3. 都没有，就把 `template` 当路径用，读外部文件不做工作区限制

没有默认模板，找不到就报错。

模板的 YAML 只放工具集，不声明变量：

```markdown
---
tools: [read, grep, bash]
---
你是一个审查者。目标一：{{target1}}
目标二：{{target2}}
{{APPENDED}}
```

`tools` 必填，必须是父会话工具集的子集，不能借子代理提权。模板里出现 `task` 直接报错，禁止递归。

---

## 5. 变量

变量靠扫描正文里的 `{{...}}` 得到，不靠 YAML 声明。

`vars` 是 JSON 对象，值必须是字符串。

规则：

- 模板里出现的每个变量，`vars` 必须给值；缺一个就报错。
- `vars` 里多出模板没有的键，也报错。
- `APPENDED` 是保留变量，不要求模板声明；模板里没有 `{{APPENDED}}` 却给了值，报错，不能悄悄丢掉。
- 不递归替换；变量值里再出现 `{{...}}` 也只当普通文本。
- 模板里 `{{` 没闭合、出现空变量名，直接报错。

`APPENDED` 用来放本次额外叮嘱，模板作者决定它出现在哪里。

---

## 6. cwd

`cwd` 由运行时给，不写在模板里。

- 主代理有 `cd` 工具：`cwd` 可给可不给；给了就用，允许工作区之外；不给就继承主代理当前 cwd。
- 主代理没有 `cd` 工具：`cwd` 参数不接受，直接用已经落盘的 cwd，避免再走一遍权限。
- 显式给的 `cwd` 必须存在且是目录，否则报错。

子会话拿自己的 `Workspace`，父的 cwd 不受影响。

---

## 7. 子会话内容

子会话继承父会话的 model 和 profile，并在 spawn 时固定到自己的 session 行上。父之后换模型、换 profile，子不受影响。

system 由三部分组成：

```text
父会话继承的 profile 系统提示词
+ 固定的 subagent 契约
+ 渲染后的模板正文
```

subagent 契约只写稳定规则：你是子代理，不能向用户提问，不能派子代理，最后用 `final submit` 收口。

子会话用一条固定的启动 user 消息开回合；具体任务已经在 system 的模板里。

---

## 8. final 工具

`final` 只给子代理，父代理没有。

它管一个内存里的稿子，不给路径，也不允许子代理自己指定文件。编辑逻辑复用 `edit` / `mass_edit` 的纯文本核心：

```text
final write content      起稿，或整篇重写
final edit old new       精确唯一匹配替换
final mass_edit edits    按行号替换
final read               看当前稿子
final submit             成稿，停止子代理
```

`final` 不重复实现 replace。精确匹配和行号匹配分别借用 `edit_text` 和 `mass_edit_text`，文件工具和 final 共用同一份业务代码。

### submit

`final submit` 不带内容，只提交当前稿子。

1. 稿子为空则报错，不停止。
2. 写到项目内统一路径：
   ```text
   <parent project>/.mypi/answers/<subsession_id>.md
   ```
3. 系统在开头加时间戳：
   ```markdown
   <!-- generated: 2026-10-22 14:03:11 (UTC+8) -->
   ```
4. 写成功才标记收口，停止循环，不再发下一次模型请求。
5. 写失败返回错误，不停止，让子代理还能重试。

子代理不写时间戳；稿子里就算有类似标记，也由系统替换，不叠加。

`submit` 最好单独调用。同一条 assistant 消息里还排着别的工具调用时，直接拒绝 `submit`。

### 没有 submit

- 有稿子：用稿子。
- 没稿子：用最后一条 assistant 文本。
- 都没有：判失败。

---

## 9. 启动顺序

1. 创建子会话行。
2. 写 task 的 toolresult 块和 `subsessions` 行，`anchor` 是新块 id。
3. 启动 `kairos now` 跑子会话。

不要先跑子代理再补块，否则它跑得快时，wake 会比 anchor 先到。

---

## 10. 生命周期

- 子代理完成或失败后停止，不删除历史。
- 停止不是消失；块、会话、final 文件都留着，方便回看。
- 父代理收到 wake 后读 final 文件。
- `subsessions` 不存 final 路径；路径按 subsession id 推出来。

---

## 11. 明确不做

- 不做 parent_id。
- 不做 `sessions.kind`。
- 不存模板、变量或子会话专属 request_meta。
- 不允许子代理自己指定 final 文件。
- 不允许递归派子代理。
- 不让 `final submit` 在写文件失败时停止。
- 不把 final 全文塞进 wake 或父会话上下文。
