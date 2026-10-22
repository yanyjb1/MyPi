# Web 设计

日期：2026-10-22

状态：定稿，可施工。

---

## 1. 定位

整个 web 只有一个工具：`web`。

它只走浏览器，不再有 HTTP 到浏览器的回落。HTTP-only 走外部 Python 脚本，不进入 MyPi 源码，也不注册成工具。

搜索、读取页面、打开页面、交互、截图，都是同一个 `web` 工具的不同操作符。

---

## 2. 工具面

```text
web {
  op: "read" | "search" | "open" | "act" | "shot",
  url?: string,
  raw?: bool,
  query?: string,
  provider?: "google" | "bing" | "duckduckgo" | "ecosia" | "startpage" | "mojeek",
  do?: "click" | "fill" | "press" | "select" | "scroll" | "eval" | "net",
  selector?: string,
  value?: string | number,
  filter?: string,
  max?: int,
  path?: string,
  full_page?: bool
}
```

### read

- 带 `url`：读该 URL，返回 markdown。`raw` 为真时返回 DOM HTML。
- 不带 `url`：读当前 work tab。
- 带 `url` 的读取使用 search 标签；不带 `url` 的读取使用 work 标签。

### search

- 只接受 `query` 和可选的 `provider`。
- 不写 `provider`：走所有能兑现当前 query 的引擎，合并、去重、按共识排序。
- 写 `provider`：只走这一家，不做共识。
- 结果上限固定 20。
- `limit`、`recency`、`engine` 都不作为工具参数。
- 时间范围用 query 里的 `after:` / `before:`。
- 指定了 provider 但它兑现不了 query，直接报错，不近似。

### open

把 work tab 导航到 `url`。

### act

在 work tab 上执行：

```text
click | fill | press | select | scroll | eval | net
```

- `click` / `fill` / `select` 用 `selector`。
- `press` 用 `selector` 当键名。
- `scroll` 用 `value` 当像素数。
- `eval` 用 `value` 当 JS 表达式。
- `net` 用 `filter` / `max`，读页面自己的 resource entries。
- `navigate` 不再作为 act 的操作；导航统一用 `open`。

### shot

对 work tab 截图。`path` 是输出路径，`full_page` 决定整页还是视口。

---

## 3. 浏览器与标签页池

整个 MyPi 进程共享一个 Chromium 进程和一个全局标签页池。

每个会话固定两个持久标签：

```text
session_id -> {
  search: TargetId,   // search 与 read url 共用
  work:   TargetId    // open / act / read 当前页 / shot
}
```

规则：

- 一个会话的 search 标签同时只跑一个搜索；多个搜索在同一标签里串行。
- `search` 和带 `url` 的 `read` 共用 search 标签。
- `open`、`act`、不带 `url` 的 `read`、`shot` 共用 work 标签。
- 标签属于会话；会话结束就关掉这两个标签。
- 标签被外部关掉时，下次调用重建。
- 全局池共享浏览器进程和标签索引，不共享页面状态。
- 子代理默认不给 `web`，避免多出标签。子代理模板里也不允许把 `web` 加回来。

没有 transient tab 作为常态。每个会话稳定只有两个标签。

---

## 4. CDP 封装

不向模型暴露原始 CDP。CDP 只是实现细节。

| web 操作 | CDP 能力 |
|---|---|
| `open` | `Page.navigate` |
| `read` | `DOM.getDocument` + `DOM.getOuterHTML`，再转 markdown |
| `act click/fill/press/select/scroll` | `Runtime.evaluate`，以后可换 `Input.dispatchMouseEvent` / `dispatchKeyEvent` |
| `act net` | `performance.getEntriesByType('resource')` |
| `shot` | `Page.captureScreenshot` |
| `search` | 浏览器打开搜索引擎，等 DOM，解析结果 |

DOM 读取优先于 `Runtime.evaluate`。反爬页面可能让 evaluate 挂住，DOM 路径更稳。

---

## 5. 搜索

### 引擎

- 默认走所有能兑现当前 query 的引擎。
- 不能兑现指令的引擎直接跳过，不近似。
- `provider` 可以钉死一家；钉死时不合并。
- Bing 的 `bing` 指国际版 `bing.com`。CN 版不支持高级语法，不作为单独 provider；地区由浏览器 profile 或 URL 参数决定，之后测试。

### 高级语法

query 支持：

```text
site:
filetype:
inurl:
intitle:
before:
after:
"phrase"
-excluded
OR
()
```

`limit` 不作为语法；结果固定取前 20。

`recency` 不作为参数；时间范围用 `before:` / `after:`。相对写法是否支持，施工时再定。

### uBO

搜索使用持久 profile。uBO 装在 profile 里，广告和赞助结果在 DOM 层被去掉。临时 profile 不用于搜索。

---

## 6. 并发与批次

- 同一会话的多个 `search` 在 search 标签里排队，串行执行。
- 不同会话各有自己的 search 标签，可以并行。
- 多个搜索调用可以同批发出；它们是独立查询，不靠调用顺序表达依赖。
- 执行和 UI 可以按完成顺序；模型上下文必须等整批全部结算后，一次性拿到所有结果。
- 结果按原始调用顺序提交，不按完成顺序。
- 某个搜索失败只影响它自己的结果；其余继续。
- 用户打断：取消未完成项，结束回合，不发起下一次请求。

---

## 7. 输出

- `read`：markdown，超长按统一输出预算处理。
- `search`：编号列表，标题、URL、摘要；必要时带说明。
- `open`：一行状态。
- `act`：一行状态；`net` 返回请求列表。
- `shot`：一行路径和字节数。
- 错误短而具体：选择器没命中、页面没打开、浏览器不可用、provider 不支持该 query。

---

## 8. Config

沿用 `config.server.agent.tools.browser.*`：

- `bin`：浏览器可执行文件。
- `port`：附加到已有浏览器。
- `profileDir`：持久 profile，uBO 和登录态在这里。
- `persistProfile`：是否保留 profile。
- `headless`：是否无头。
- `userAgent`：覆盖 UA。
- `extraArgs`：额外启动参数。

搜索不新增配置键。引擎列表和 provider 名称是代码内枚举。

---

## 9. 未决

- Bing 国际版在实际 profile 下的高级语法表现，施工后测试。
- 每个引擎的浏览器-only 解析路径是否都能稳定工作。
- 相对时间写法是否加入 query 语法。
- 全局浏览器并发上限的具体数值。
- 子代理完全不给 `web` 后，是否需要只读的搜索替身；当前不做。
