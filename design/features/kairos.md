# Kairos 异步设计

日期：2026-10-22

状态：核心设计收口。

---

## 1. 名字

只有一个工具，叫 `kairos`。

不是三个工具。立即执行、延迟一次、按 cron 重复，都是 `kairos` 的模式：

```text
kairos now     立即在后台执行
kairos after   延迟一次
kairos every   按 cron 表达式重复
```

`kairos` 是希腊语里的“恰当的时机”，和 `chronos` 那种钟表时间相对。它不只是一个计时器，而是“在合适的时刻做某件事”。

这个工具纯给 AI 用，没有人类 `/kairos` 命令。

---

## 2. 生命周期

所有 `kairos` 任务都只活在当前进程的内存里。

- 进程停，全部消失。
- 会话关，属于它的全部取消，正在跑的命令进程组杀掉。
- resume 不扫描历史恢复任务。
- 不补跑错过的时点。

它们产生的 wake 和结果作为普通块落库，所以历史里看得到；但不会拿这些块把任务重新装起来。不是 todo 那种从历史重放状态的机制。

---

## 3. 参数

```text
kairos {
  op: now | after | every | list | status | result | cancel,
  type?: command | text,
  content?: string,
  timeout?: duration,
  delay?: duration,
  expr?: string,
  count?: integer,
  id?: string
}
```

按 `op` 取用：

- `now`：`type`、`content`、可选 `timeout`。
- `after`：`type`、`content`、`delay`。
- `every`：`type`、`content`、`expr`、可选 `count`。
- `list`：无。
- `status` / `result` / `cancel`：`id`。

`type = command` 时，`content` 是一条 shell 命令。
`type = text` 时，`content` 是纯文本，到点直接当提醒唤醒。

---

## 4. now

立即在后台执行，不占当前回合。

- 返回任务 id 和一个回执。
- `timeout` 不填默认 1 小时；可以填很大的数，只要会话还活着。
- 超时杀掉整个进程组，然后唤醒报超时。
- 完成或失败都唤醒，只报状态；结果由模型调用 `kairos result <id>` 取。
- 结果过长时走现有 artifact 机制，落成 `#<ref>`。

`type = text` 对 `now` 也成立，但语义就是立即完成并唤醒，实际用途不大。延迟提醒应该用 `after`。

---

## 5. after

一次性延迟。

- `delay` 只接受相对时长，比如 `10min`、`2h`、`1d`、`1h30m`。
- 不接受“明天八点”这种绝对时间；绝对时点用 `every`。
- 到点后：
  - `type = command`：创建一个后台任务执行命令，完成或超时后唤醒。
  - `type = text`：直接产生一条文本唤醒。
- 只响一次，没有下一次，所以没有重叠问题。

---

## 6. every

按 cron 表达式重复触发。

- 五段：分、时、日、月、周。
- 支持 `*`、数字、列表、范围、步长。
- 例如每周三上午八点：`0 8 * * 3`。
- 按系统本地时区计算下一次触发。
- 进程活着才有效；重启不补跑。

### count

- 不填默认 3。
- `-1` 表示无限。
- `0` 和其他负数拒绝。
- `count` 只算真正启动的次数，跳过不扣。
- 启动一次成功，`count` 减一；减到 0 就从调度器移除，已经跑着的那次让它跑完。
- `type = text` 每次触发也算启动一次。

### 重叠

重叠只发生在同一条 `every`：上一次触发的命令还没结束，下一次又到点了。

默认跳过这一次：

- 不启动新进程。
- 第一次跳过立刻发 wake，说明原因。
- 后续跳过只累加 `skipped`，不再每条都唤醒。
- 下一次真正启动，或上一次任务结束时，把累计的跳过次数带进消息。
- `skipped` 在真正启动一次后清零。

不同 `every` 之间可以同时跑，不算重叠。所有任务共用一个全局并发上限；池满时跳过并说明是池满，不是单条 `every` 重叠。

---

## 7. 唤醒

`now`、`after`、`every` 共用同一套唤醒。

- 会话空闲：开一个新回合。
- 会话活跃：在下一次模型请求前插一条 wake，不打断当前流。
- wake 只报状态，不搬结果；结果用 `kairos result` 取。
- 多个任务同时完成，按会话合成一条 wake，只开一个回合。

---

## 8. 结果

`kairos result <id>` 返回任务的完整输出。

- 输出过长时，走现有 front door 的 spill/artifact 机制，结果里给 `#<ref>`。
- 任务自己只在内存或临时文件里保留输出，取走后可以释放。
- `status` 看状态、退出码、耗时、输出大小。
- `list` 看当前会话的任务和排程。
- `cancel` 取消任务或排程；取消不产生完成唤醒，除非调用方要求。

---

## 9. 安全

`type = command` 的 `now`、`after`、`every` 都要过 guard：

- 创建时先粗筛一遍。
- 执行时对最终命令再过一遍。
- 执行时那次才是最终裁决。
- alias 展开、`#<ref>` 重写都在 guard 之前完成。

`type = text` 不执行命令，只产生唤醒。

---

## 10. 明确不做

- 不做跨进程、跨生命周期的任务。
- 不做 resume 恢复。
- 不做错过的时点补跑。
- 不做三个独立工具；只有 `kairos` 一个工具。
- 不把调度状态写进数据库。
- 不让 `type = text` 执行命令。
- 不让 `every` 的重叠任务排队堆积；默认跳过。
