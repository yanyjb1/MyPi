# Alias 设计

日期：2026-10-22

状态：核心设计收口，进入施工前细化。

---

## 1. 定位

alias 是给模型用的命令快捷方式，作用相当于一份由模型自己维护的 bashrc。它只做别名，不引入变量，不提供 shell 函数、环境变量、启动脚本这些能力。

目的只有一个：把反复出现的长命令压成一个短名字，省 token。安全边界仍然在 bash 的 guard，不在 alias 本身。

---

## 2. 作用域

alias 不按会话 id 分，按块分。

每条 alias 有一个 `anchor`，就是写入它的块 id。每次 bash 执行时，从当前会话的块列表里收集可见 alias；只要这个块在当前会话里，这条 alias 就可见。会话是主会话还是别的形态，不在 alias 层判断。

这样模型只能看到自己这条路径上出现过的块所带的 alias，不会看到它不知道的块里的东西。

---

## 3. 表

```sql
CREATE TABLE aliases (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    anchor   INTEGER NOT NULL,   -- 写入它的块 id
    name     TEXT NOT NULL,
    command  TEXT NOT NULL,
    ts       TEXT NOT NULL
);

CREATE UNIQUE INDEX aliases_by_anchor
    ON aliases(anchor, name);
```

字段：

- `id`：写入顺序。
- `anchor`：这条 alias 的锚点块。
- `name`：别名，第一个 token。
- `command`：展开后的命令模板。
- `ts`：写入时间，格式同 memo，例如 `2026-10-22 14:03:11 (UTC+8)`。

同一 `(anchor, name)` 不允许重复。不同 anchor 可以有同名 alias；执行时取当前可见集合里最新的那条。

---

## 4. 展开规则

只展开整条命令的第一个 token，并且只展开一层。

```text
clean foo
    -> rm -rf ./tmp/* foo
```

规则：

- 第一个 token 精确匹配 alias 名才展开。
- 参数原样接在 alias 命令后面，中间补一个空格。
- 展开结果不再继续找 alias，避免循环和不可预测的递归。
- alias 命令不能为空，不能包含换行。
- alias 名字不允许空格，只允许 ASCII 字母、数字、下划线、横线，首字符是字母。
- 不提供 `$@`、`$1` 这类占位符；参数就是简单追加。
- 不支持变量定义。alias 命令里可以有普通的 shell 语法，但不提供“管理变量”的功能。

执行顺序固定为：

```text
alias 展开
    -> #<ref> 重写
    -> guard（看展开后的完整命令）
    -> 执行
```

`#<ref>` 必须在 alias 展开之后重写，因为 alias 命令本身可能带 artifact 引用。

---

## 5. 两段 guard

### 写入时

`alias add` 先对 alias 的 `command` 过一遍 guard。

- `Block`：拒绝写入，alias 不落库。
- `Warn`：允许写入，按现有规则处理警告。
- `Allow`：写入。

这次检查是粗筛，拦住固定就危险的 alias，例如 `nuke -> rm -rf /`。它不能替代运行时检查，因为参数和 cwd 都可能变。

### 执行时

bash 工具展开 alias、重写 `#<ref>` 之后，对最终命令再过一遍 guard。

- `Block`：拒绝执行，命令不跑。
- `Warn`：执行并给结果打警告标签。
- `Allow`：执行。

这次才是最终裁决。`clean` 展开成 `rm -rf ./tmp/*` 可以放行；`clean /` 展开成 `rm -rf ./tmp/* /` 必须拦住。alias 不允许绕过 guard。

---

## 6. 写入路径

AI 通过 `alias` 工具维护 alias。工具不是实际写入者，真正的写入由 writer 完成：

1. 工具返回一个待写入的 alias 操作。
2. writer 在写 toolresult 块的同一个事务里插入 alias 行。
3. `anchor` 就是刚拿到的 toolresult 块 id。

这样块和 alias 要么都成功，要么都回滚，不会出现“工具说写了、alias 没有”的情况。

同一个回合内，`alias add` 通过后要立即更新 `Env` 里的 alias map，后续 bash 调用马上能用；落库仍然按上面的路径走。

---

## 7. 运行时 alias map

bash 工具不直接查数据库。每个回合由 driver 把当前会话可见的 alias 装进 `Env`，bash 只读这份 map。

- 回合开始时按当前会话的块列表收集 alias。
- 同一名字取最新一条。
- `alias add` 在当前回合内立即更新 map。
- 下一回合重新从数据库构建。

可以按 alias 集合的哈希做缓存；缓存只是加速，不改变可见性规则。

bashrc 可以生成，作为缓存或给人看的产物，但执行入口是 Rust 展开后的命令。bash 不负责展开 alias，guard 也永远看展开后的命令。

---

## 8. 工具面

核心操作：

```text
alias add name command
alias list
alias remove name
```

具体参数名和返回文本在施工时定。`add` 必须先过 guard；`remove` 只影响当前可见集合里的 alias，具体按块删除还是追加删除记录，施工时按存储路径定。

---

## 9. 明确不做

- 不做变量管理。
- 不做 shell 函数。
- 不做全局 alias；alias 只按块绑定。
- 不让 bash 自己展开 alias 后再交给 guard；guard 必须看展开后的命令。
- 不让 alias 绕过 guard。
- 不支持多行 alias。
- 不支持递归 alias。
- 不在 alias 层判断会话形态；只看块在不在当前会话里。
