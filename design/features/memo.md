# Memo 设计

日期：2026-10-22

状态：设计暂定，先按这份实现；alias 设计另开文档。

---

## 1. 定位

memo 是人和 AI 共用的一套命名笔记。数据在全局数据库里，跨会话、跨进程存在。

它有两种归属：

- `current`：属于一个块，不属于会话。
- `global`：不属于任何块，所有会话可见。

因为归属不同，`current` 和 `global` 可以出现同名 memo，互不冲突。

---

## 2. 表

只有一张表，只追加：

```sql
CREATE TABLE memos (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    anchor   INTEGER,          -- NULL = global；否则是块 id
    name     TEXT NOT NULL,
    content  TEXT NOT NULL,
    ts       TEXT NOT NULL
);
```

字段含义：

- `id`：写入顺序，也是 history 的顺序。
- `anchor`：这条 memo 的锚点。`current` 是块 id，`global` 是 `NULL`。
- `name`：memo 名字。不允许空格，命令按空格切分。
- `content`：这一版的完整内容。
- `ts`：写入时间，直接存成品字符串，例如 `2026-10-22 14:03:11 (UTC+8)`。用系统时区。

约束：

```sql
CREATE UNIQUE INDEX memos_by_anchor
    ON memos(anchor, name) WHERE anchor IS NOT NULL;

CREATE UNIQUE INDEX memos_global_name
    ON memos(name) WHERE anchor IS NULL;
```

`anchor` 是 memo 的身份锚点，不是每次写入的块。`add` 时确定，`mod` 沿用，`del` 按它删。同一条 memo 的所有版本共享同一个 `(anchor, name)`。

---

## 3. 操作

### add

插入第一行。`current` 必须带一个已经存在的块 id；`global` 的 `anchor` 为 `NULL`。

同一 `(anchor, name)` 已存在且未被删除时，`add` 报错，要求改用 `mod`。

### mod

修改当前内容。语义上是替换，存储上是给同一个 `(anchor, name)` 追加一条新记录，不覆盖旧行。

- `cat` 看到新内容。
- `history` 仍能看到旧内容。
- `anchor` 和 `name` 不变。

### cat / view / display

同一个操作的三个名字。取同一个 `(anchor, name)` 下 id 最大的那行，也就是当前内容。

命令补全池要认这三个别名。`Call::Pools` 目前只给模型和 profile；memo 的命令别名要么扩 `CandidatePools`，要么由命令层自己带一份，和 alias 的命令补全一起处理。

### history

取同一个 `(anchor, name)` 的全部行，按 id 升序。删除后历史不再存在，因为 `del` 是物理删除。

### del

删除同一个 `(anchor, name)` 下的全部行，也就是这个 name 的所有 entry。删除后 history 不再保留旧版本。

### list

列出名字。

- `current`：取当前会话列表里那些块对应的 anchor，再按 `(anchor, name)` 去重。
- `global`：取 `anchor IS NULL` 的名字。

---

## 4. 人类写入

人类通过 `/memo` 命令操作，`current` 和 `global` 都能用。

### current 的锚点

`current` 的锚点是**当前会话列表里最后一个已经落库的块 id**，不是会话 id，也不是流式预览里的块。

有对话时，前端向 session 要会话头。历史区是显示层，可能只加载了本地一段，不能拿“当前可见的最后一个块”当锚点。session 的结构缓存有全量结构，直接给会话头。

回合正在跑时，只取已经落库的最后一个块；拿不到就拒绝 `current`，不要拿还没落库的预览块当锚点。

### 没有对话时

还没有对话、甚至还没有 `init` 时，没有块可锚：

- 不填 scope，默认 `global`。
- 显式写 `current`，报错，拒绝无中生有。
- `global` 不需要会话，也不需要 `init`，可以直接写。

### Post 形状

人类写入走 post，锚点显式带上：

```text
Post::Memo {
    op, scope, anchor: Option<i64>, name, content
}
```

- `add` + `current`：前端从 session 拿会话头，填 `anchor`。
- `add` + `global`：`anchor = None`。
- `mod` / `del` / `cat` / `history`：anchor 来自已有 memo，不是会话头。

服务端校验 `anchor` 确实是这条会话列表里的块，不能随便塞一个块 id。

### current 下的同名

`current` 下同名只允许一个 anchor。`add` 遇到当前会话里已有同名就报错，`mod` / `del` 用已有那条的 anchor。

如果以后要允许不同 anchor 同名，`cat` / `mod` / `del` 就必须多一个 anchor 参数；当前不做。

---

## 5. AI 写入

AI 通过 `memo` 工具操作，scope 只能是 `current`，不能碰 `global`。

AI 的 memo 工具不是实际写入者。它只返回一个占位结果；真正的写入由 writer 完成：

1. 工具返回一个待写入的 memo 操作。
2. `hand_results` 写 toolresult 块。
3. writer 在同一个事务里插入 memo 行，`anchor` 就是刚拿到的 toolresult 块 id。

这样块和 memo 要么都成功，要么都回滚，不会出现“工具说写了、memo 没有”的情况。工具返回什么不重要，事务成了，memo 就在。

实现上给 `ToolOutput` 加一个 side effect 字段，例如 `writes` 或 `effects`；不要复用现有的 `memo` 字段，那个是给前端的结构化 payload。

---

## 6. 删除会话与导出

删除会话时，如果这条会话有 `current` memo，先导出到下载文件夹，再随会话处理。

- 格式：markdown。
- 排序：按每个 memo 第一次写入的时间排序，再排它自己的变迁。
- `global` 不随会话删除，也不导出。

导出文件粒度尚未拍板：当前倾向一个文件、一个 memo 一节；如果以后要一个 memo 一个文件，再改。

---

## 7. 时间

`ts` 直接存成品字符串，例如：

```text
2026-10-22 14:03:11 (UTC+8)
```

用系统时区。排序不依赖这个字符串，统一按 `id`；`id` 就是写入顺序。

---

## 8. 明确不做

- 不把 `current` 绑到 session id。
- 不把 memo 工具当成实际写入者。
- 不为 `del` 保留 tombstone；删除就是删除。
- 不给 AI 开放 `global`。
- 不在 memo 表里存时区字段；时间已经写成成品。
- 不让 memo 名字包含空格。
