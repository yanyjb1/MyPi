# CDP 客户端 — 方案与说明

先说结论：**可以做，工作量可控，我建议做成一个独立 `browser` 工具**，模型用一条命令就能驱动浏览器，不需要手搓 CDP 协议细节。

---

## 一、Helium 核实结果

| 项目 | 实测结果 |
|---|---|
| 路径 | `/opt/helium/helium`（另有 `helium-wrapper`、`chromedriver`） |
| 版本 | Helium 0.17.2.1（Chromium 153.0.8010.52） |
| CDP 支持 | ✅ 完整支持，`--remote-debugging-port=0` 启动后，`DevToolsActivePort` 文件会写进 `--user-data-dir` |
| 握手验证 | `/json/version` 返回 `Chrome/153.0.8010.52, Protocol-Version 1.3`，WebSocket 握手成功 |
| 实测能力 | `Runtime.evaluate`（取 title、innerText、navigator.userAgent）、`Page.navigate`、`Page.captureScreenshot`（PNG 772×510）全部跑通 |
| 无头模式 | `--headless=new` 正常，截图/求值均可 |
| 附带 | `/opt/helium/chromedriver` 也存在（Selenium 路线可用，但不需要） |

也就是说：Helium 就是标准 Chromium，CDP 客户端写好后对它完全通用，以后换成 Chrome/Edge/Brave 也能连。

---

## 二、协议是什么、复杂在哪

CDP 全称 Chrome DevTools Protocol，本质是 **WebSocket 上的 JSON-RPC**：

```
连接 ws://127.0.0.1:<port>/devtools/page/<id>
发 {"id":1,"method":"Runtime.evaluate","params":{"expression":"1+1","returnByValue":true}}
收 {"id":1,"result":{"result":{"type":"number","value":2}}}
收 {"method":"Page.loadEventFired",...}        ← 事件（没有 id）
```

三层事实（全部用 Python 裸 WS 客户端实测确认，非抄文档）：

1. **发现**：浏览器启动时把端口写进 `<user-data-dir>/DevToolsActivePort`，第 1 行是端口号，第 2 行是浏览器级 WS 路径。同时 `http://127.0.0.1:<port>/json/list` 能枚举所有标签页。
2. **连接**：标准 WebSocket 握手（`Sec-WebSocket-Key` + SHA1 + `258EAFA5-E914-47DA-95CA-C5AB0DC85B11`），之后双向发文本帧。
3. **请求/响应配对**：请求带自增 `id`，响应带相同 `id`；事件没有 `id`，会穿插在响应之间到达 —— 所以客户端必须**后台持续读**，否则事件堆积会阻塞响应（实测踩过：同步等 id 会超时，因为前面堆着事件）。

CDP 的“难点”不是协议本身，是它**域多、命令多**（几百条），手搓全量客户端不现实也没必要——你只需要模型常用的那几条。

---

## 三、封装到什么程度（这是你问的核心）

我的建议：**三层结构，每层薄**。

```
┌─ 工具层   browser 工具（给模型用的唯一入口）─────────────┐
│  参数: {"command": "navigate|eval|click|screenshot|…", "url"/"js"/…} │
│  返回: 纯文本，直接进对话上下文                             │
├─ 会话层   CdpSession（维护一个标签页的连接）────────────────┤
│  call(method, params) → Value  （id 配对 + 后台事件泵）     │
│  wait_event(name, timeout)                                 │
├─ 传输层   WS 连接 + 浏览器生命周期（launch/find/connect）──┤
└────────────────────────────────────────────────────────────┘
```

关键取舍：

**1. 不要生成全量协议绑定。**
社区做法（`chromiumoxide` 之类）是用 Chromium 的 `browser_protocol.json` + `js_protocol.json` 生成几百个 Rust 结构体。你的场景用不上——模型需要的就那十几个动作。手写这十几个就够了，编译快、依赖少、出问题好排查。

**2. 不要暴露裸 CDP 给模型。**
让模型直接发 `{"method":"Runtime.evaluate",...}` 也能工作，但模型会写出奇怪的方法名/漏参数，然后你调试协议而不是调试页面。**把常用动作固化成命令枚举**，模型填参数就行：

| command | 参数 | 干什么 |
|---|---|---|
| `navigate` | `url` | 打开 URL，等到 load |
| `eval` | `js` | 执行 JS，返回值序列化成文本 |
| `click` | `selector` | `document.querySelector(sel).click()` |
| `type` | `selector`, `text` | 聚焦输入框、逐字输入 |
| `screenshot` | `path?` | PNG 截图存文件 |
| `tabs` | — | 列出所有标签页 |
| `new_tab` | `url` | 新开标签页 |
| `close_tab` | `tab_id` | 关标签页 |
| `scroll` | `dy` | 页面滚动 |
| `content` | `selector?` | 取 innerText / innerHTML |

这套覆盖了日常自动化 90% 场景。真遇到奇怪的，加一个 `raw` 命令兜底（发任意 method+params），但文档里标注“仅排查用”。

**3. 会话生命周期：进程内单例 + 惰性启动。**
- 第一次调用 `browser` 工具时才启动 Helium（headless 或 headed 由配置决定），之后复用。
- 工具自己管 `--user-data-dir`（默认指向固定目录，重启后保留 cookie/登录态），**不要用 `mktemp -d`**——那样每次调用都是全新浏览器，登录态丢失。
- 浏览器挂了自动重启一次；再失败把错误原样返回给模型。

**4. 事件必须后台读。**
这是我实测踩的第一个坑：如果只等响应、不读事件，第二条命令就会超时（事件把 TCP 流堵住了）。Rust 里用一个线程（或 async 任务）专门收消息、按 `id` 分发即可。

**5. 超时与返回长度。**
- 每条命令默认 10s 超时，`navigate` 等 load 事件 20s。
- `eval` 返回值必须**截断**（比如 4KB），不然一个 `document.body.innerText` 就能把上下文撑爆。截图落盘不进上下文，只返回路径。

---

## 四、技术选型

| 选项 | 结论 |
|---|---|
| 手搓 WebSocket（RFC 6455） | 可行（Python 里我 40 行就写完了），Rust 里也就 150 行左右。**依赖最少**，但要自己处理分片、掩码、关闭帧 |
| `tungstenite` 0.30（同步） | 业界标准、纯 Rust、无 async 运行时依赖，跟你现有的 ureq 同步风格一致 ✅ **推荐** |
| `chromiumoxide` / `headless_chrome` | 全量 CDP 绑定，重、tokio 依赖，对你这个场景是杀鸡用牛刀 ❌ |

`tungstenite` + 你已有的 `ureq`（做 `/json/list` 的 HTTP 发现）就够了，都不引 tokio。

---

## 五、工作量估算

| 部分 | 行数（估） | 说明 |
|---|---|---|
| 传输层（WS + 发现 + 进程管理） | ~250 | `tungstenite` + `ureq`，进程 spawn/kill，DevToolsActivePort 解析 |
| 会话层（call/id 配对 + 事件泵） | ~120 | 后台线程 + `mpsc`/`Mutex<HashMap<id, sender>>` |
| 工具层（命令枚举 + 参数解析 + 文本化） | ~300 | 注册进 `BuiltinTools`，错误转成模型能读的字符串 |
| 测试 | ~150 | mock 一个假 CDP 服务端，测 id 配对/超时/截断 |
| **合计** | **~800** | 一次 PR 内可完成，不算大 |

---

## 六、安全边界（必须说清楚）

浏览器工具本质是**把任意网页 JS 执行权交给模型**，比 bash 危险得多，需要明确规则：

- 默认 headless + 独立 `user-data-dir`（跟你日常浏览器隔离，cookie 不互通）
- 是否允许 `file://` 访问？建议禁止
- 是否允许访问内网地址（`http://localhost:*` / `192.168.*`）？建议默认禁止，避免模型借浏览器绕过 bash_guard 的网络限制
- `bash_guard` 拦截危险 shell 命令的思路要复用：对 `eval` 的 JS 做**关键词黑名单**不现实（JS 太灵活），改成**在工具层只开放白名单命令**（`navigate/eval/click/...`），而 `raw` 默认关闭，开了就日志记录

---

## 七、建议的推进顺序

1. **先做传输层 + 会话层**（不接模型），写成独立库模块，用 `cargo test` 验证 connect → evaluate → navigate → screenshot 全链路（对照我实测的 Python 版本，协议行为已知）
2. **再接 `browser` 工具**，只开放 `navigate / eval / content / screenshot / tabs` 五个命令，跑通真实页面
3. **补齐交互类命令**（click/type/scroll/new_tab），这一步开始要处理元素定位失败的报错格式
4. 最后加配置项（`browser.headless`、`browser.profile_dir`、`browser.allow_hosts`）

每一步都可以独立验证，不会一次性吞 800 行。

---

要不要按这个方案直接开工？如果确认，我建议从第 1 步开始，先给你看传输层 + 会话层的接口定义，认可了再往下写。
