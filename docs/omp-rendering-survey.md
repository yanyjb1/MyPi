# omp (oh-my-pi) 聊天区渲染调查报告 — MyPi 抄作业参考

> 范围：`/tmp/oh-my-pi/packages/tui/src`（18.2.7）。只看聊天区渲染链路，不碰 status-line、overlay、/model 向导等。
> 结论先行：**omp 的所有"魔法"在 Rust 里都有更便宜的等价物，因为我们本来就是 Rust——它最难的部分（折行、宽度、高亮）在 pi-natives 里本来就是 Rust 写的，我们直接用同款 crate 即可，零 FFI 成本。**

---

## 一、总架构：omp 怎么画一帧

```
Component 接口: render(width) -> readonly string[]   // 一切组件只产出"带 ANSI 的字符串行"
   └── Container: children 数组，自上而下逐行拼接
        └── TUI (3622 行): 拿到整棵树的行数组后做差分上屏
```

核心事实（`tui.ts`）：

1. **组件树每次全量求值，但上屏是行级 diff**（`tui.ts:2862`）。每帧对 viewport 每行比较新旧 prepared 行，只重写变化的行（`\x1b[{row};1H...`），不变行 `continue`。历史区（滚动进 scrollback 的部分）**从不重绘**。
2. **块生命周期协议**（`chrome/transcript-container.ts`）：`active → settled → committed`。active 块（正在流式的那条）每帧重渲；settled 后进 `appendOnly` 契约——稳定行只许追加、禁止改写；committed 行物理上已进终端 scrollback，渲染器直接不再管它。
3. **活跃块上限** `MAX_LIVE_BLOCKS = 256`，超过压力回收。这就是它长会话不炸的原因：**重绘成本只与"屏幕上可见的活跃块"成正比，与会话总长无关**。
4. 用户评价"它从上到下渲染很笨"——对，组件树求值是自上而下的（`Container.render` 顺序遍历 children），这是全量求值路径；但 diff 上屏把浪费抵消了大半。我们已有的"自底向上视口 + LRU 块缓存"在这点上**比它先进**，不用改。
5. SGR 相邻合并（`tui.ts:577-701`）：`\x1b[39m` 后紧跟 `\x1b[38;2;r;g;b` 合并成一条。据注释约 40% 的 SGR 序列可合并，显著降低字节量。这是它对"慢终端"的优化，ratatui 自己就做等价的事，我们不用抄。

## 二、Markdown 渲染（`components/markdown.ts`，3738 行——全项目最重）

**技术选型**：解析器是 **marked 的 fork**（`@oh-my-pi/pi-utils/marked/core.ts`，1595 行，vendored 进仓库，非 npm 依赖）。产出 token 树，然后**自己渲染成行数组**——不是 marked 的 HTML renderer，是纯 terminal renderer。

关键行为（全部可 1:1 移植）：

| 元素 | 行为 | 位置 |
|---|---|---|
| h1 | **加粗+下划线**，无 `#` 前缀；支持终端 text-sizing 时用双倍高字符 | markdown.ts:2911-2929 |
| h2 | 加粗，无前缀 | 同上 |
| h3+ | 加粗 + 原始 `### ` 前缀 | 同上 |
| 段落 | 折行后块间空一行（下一个 token 是 space/list 时不加） | :2947 |
| 列表 | `- `/`N. ` 前缀（ordered 用 `token.start` 起算），**悬挂缩进**：续行对齐正文而非前缀（`10. ` 是 4 格） | :3287-3359 |
| 引用 | 每行 `│ ` 前缀（quoteBorder 色），整块先渲染再统一折行折行时重施样式 | :3086-3104 |
| 围栏代码 | ` ```lang ` 行和 ` ``` ` 行都用 mdCodeBlockBorder 灰色；**内部走语法高亮** | :2978 |
| 行内代码 | mdCode 紫色 `#e5c1ff` | tui-adapters.ts:208 |
| 链接 | 文字 mdLink 蓝 + URL mdLinkUrl 暗灰 | 同上 |
| math | LaTeX → Unicode 符号（我们不做，跳过） | :2949 |
| mermaid | ASCII art 化（我们不做，跳过） | :2964 |

**它吃有序/无序列表**——你吐槽我们现在不吃的那个——实现就是上面那张表，`marked` lexer 出 `list` token（含 `ordered`/`start`/嵌套 `items`），渲染端递归 `depth` 缩进。

**性能架构（抄的重头）**：
- L1：实例内 `#cachedText/#cachedWidth/#cachedLines`——同宽度同文本直接返回行数组引用。
- L2：模块级 LRU 跨实例共享渲染结果。
- **流式增量**（这是精华，`setText` + `#streamPrefix*`）：
  1. append-only 增长检测（`text.startsWith(old)`）；
  2. marked 无断点续 lex，但**块 tokenization 在 `\n\n` 边界是局部的**——它缓存"最后一个空行边界前的冻结前缀 token"，每帧只 re-lex 新增尾巴，把流式 reveal 从 O(N²) 降到 O(N)（markdown.ts:1755-1761 注释写明）；
  3. 冻结前缀的行进 per-token 行缓存（PoC H）；未冻结尾巴里的代码块用 **stateful HighlightStream** 增量高亮（`push()` 携带 syntect ParseState，跨 chunk 结果 = 整块一次高亮，逐字节一致，:2635-2677）。
- 流式中的普通文本尾部**不高亮不折行缓存**，只有完成行才高亮。

**Rust 等价物**：`pulldown-cmark`（event 流）或 `comrak`（AST）。pulldown-cmark 的 `Parser` 本身就是惰性迭代器 + 可 `into_offset_iter`，天然支持"从上次偏移继续"——比 marked 更适合流式增量，**不需要** fork 解析器。列表/引用/代码块 token 全有。

## 三、语法高亮（Rust 老熟人）

**后端 = syntect 5.3**（`crates/pi-natives/src/highlight.rs`，738 行，feature：`default-syntaxes + default-themes + regex-fancy + yaml-load`——regex-fancy 是纯 Rust 正则引擎，不需要 onig C 库）。

- scope → 11 个语义色映射：comment/keyword/function/variable/string/number/type/operator/punctuation/inserted/deleted（highlight.rs:1-10）。**不是**每 scope 直出主题色，而是收敛到 11 类，主题只给 11 个 ANSI 序列。这个收敛映射直接抄，`syntaxComment #6A9955` 等值在 dark.json:60-71。
- 额外 vendor 了 Julia/Nix/Mermaid/TS/TSX/Astro 六个 sublime-syntax（highlight.rs:32-39）。
- `HighlightStream`：跨 push 保持 `(ParseState, ScopeStack)`，流式代码块逐行 push。
- 缓存：`highlightCached` LRU + thread-local scope→color 表。
- **我们已经在用 syntect**（`tui/highlight.rs`）——零新依赖，只需把"scope → 11 语义类"这层映射和流式 ParseState 搬过来。

## 四、工具卡片（`render/` 域，~1500 行）

两种拓扑（`tool-card.ts:16-20`）：
- **framed**：自绘圆角框。`╭─ 标签 ─...─╮` / 内容行 `│ ... │` / `╰──╯`（`output-block.ts:74-188`）。分隔线用尖角 tee。
- **plain**：无框内联 + 状态底色（pending/success/error 三色底）。

边框色规则（output-block.ts:78-88）：显式指定 > error 色 > warning 色 > running/pending 时 accent > 其余 dim。
状态底色（utils.ts:105-109）：success→`#161a1f`，error→`#291d1d`，pending→`#1d2129`。

标题行格式（`render/status-line.ts:33-55`）：
```
{icon} {title}: {desc} · [badge] · {meta}
icon = 状态符号或 spinner 帧; title=accent; desc=muted; badge=着色方括号; meta=dim
```
状态符号（render-utils.ts:276-299）：success→`✓`、error→`✗`、warning→`⚠`、pending→`○`、running→**spinner 当前帧**、aborted→`■`。每个工具还有身份 glyph（symbols.ts:627+）：bash=`❯`、edit/write=`✎`、read=`📄系`、browser=`🌐`、search=`⌕`。

Spinner：盲文 `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`，80ms/帧，**所有并行工具块共享一个 ticker 同相位**（tool-execution.ts:184-234，一个 setInterval 驱动 N 块）——这个设计值得抄，我们现在是 App 里一个 spinner 字段，天然单相位，等价。

工具结果缓存：bash 卡把"输出切分+截断"的结果做成 per-instance snapshot 缓存，键 = (width, expanded, raw…)（bash.ts:270-310），否则每次按键重跑全 transcript 的输出切分（issue #2081）。**这对应我们的 BlockCache，我们已有等价物。**

## 五、助手块 & thinking 动画（`chat/assistant-message.ts`，1214 行）

- 正文/思考按 content 顺序渲染；思考正文 = 斜体 + `thinkingText` 灰色的 Markdown（:1137-1143）。
- **思考脉冲动画**（隐藏思考时）：
  - 8 帧花型 `✻ ✼ ❉ ❊ ✺ ✹ ✸ ✶`（:86）；
  - **余弦缓动帧驻留 70–230ms**（呼吸感，不是固定间隔；:532-535）；
  - 标签 = thinkingText 色花型 + muted `" Thinking"`；
  - **速度徽标**：真实出 token 时显示 `· {total} · {rate} toks/s`，rate 徽标颜色从 dim 灰按 `sqrt(rate/200)` 插值到 accent 色（:510-520，truecolor 时 `lerpHex`）。滑动窗口 3s 统计速率（SPEED_WINDOW_MS=3000）。
  - 只在"思考是活跃尾块且未定稿"时动画（:487-496）。
- 流式 LIVE 徽标 `● LIVE <n>`，thinkingHigh 紫 `#b281d6`（status-line/segments.ts:763）。
- 错误内联块：error 色，默认封顶 8 行 + ctrl+o 展开全文（:800-812）。

## 六、用户气泡（`chat/user-message.ts`）

- 整块背景 `userMessageBg #221d1a`（暖黑，和普通终端底有区别），前景 `userMessageText`。
- 内部唯一子组件 = Markdown(padding 1,1)，**气泡内也吃完整 markdown**。
- 背景实现：每行 `theme.bg(...)` 包裹 + 行尾续接背景 SGR 抗重置（box.ts:89-100）。
- OSC 133 shell-integration 标记包首尾行（终端 jump 支持，可不做）。

## 七、系统消息家族（chat-transcript-builder.ts 分发表，:266-372）

omp 会往 transcript 里发的非对话块：

| 类型 | 形态 |
|---|---|
| turn 用量行 | turn 结束时 `Spacer(1) + MetricRow`（dim 色：tokens · cost · duration · ttft）（usage-row.ts:98-118） |
| compaction/branch summary | FramedMessageComponent 框卡：`customMessageBg` 底 + 边框 + 图标标题 + markdown 正文，可折叠 |
| hook/custom 消息 | 同上共享 frame（chrome/message-frame.ts），extension 可自定义 renderer |
| cache 失效标记 | `⊘ cache miss · N tokens`（muted 标签 + dim 短线）（cache-invalidation-marker.ts:78-93） |
| served-model 标记 | 同构分隔线 |
| 错误横幅 | pinned 在编辑器上方：上下 error 色 DynamicBorder + 加粗首行，封顶 4 行 |
| fileMention | 文件列表块 |
| todo 提醒 | agent 停止但 todo 未完 → warning 图标 + 斜体未勾选列表 |

## 八、主题机制（决定"色从哪来"）

- 所有组件只经 `theme.fg(token)/bg(token)` 拿色，token 集合 ~40 个（schema.ts:26-86）。dark/light JSON 预设 + 运行时 hex→ANSI 缓存。
- **themeEpoch 单调计数器**：任何组件缓存键里都埋它，主题一换全体失效（theme.ts:383-391）。我们没有运行时换主题需求，可以不做。
- accent 色：会话名 djb2 → 色相弧避让 → OKLCH 尖峰重归一（session-color.ts，报告文件第五节有完整公式）。好看但复杂，**建议不抄**——我们单会话不需要 per-session accent。

## 九、Rust 移植评估（核心问题逐条回答）

### 9.1 能不能 1:1 复刻？
**能，且每一样都有直接等价物：**

| omp 用 | Rust 等价 | 状态 |
|---|---|---|
| marked fork | `pulldown-cmark`（零依赖 parser，纯 Rust，Comformance 100%） | 新增 1 个依赖，~2 crate 无传递重依赖 |
| syntect + regex-fancy | 同款 syntect | **已在用** |
| 自写 ANSI 折行 `wrap_text_with_ansi`（pi-natives 里本来就是 Rust，UTF-16 切分、ANSI 状态跨行携带、宽字符感知） | 同逻辑直接抄实现思路或用 `textwrap` + `unicode-width`（我们已用 unicode-width） | 抄语义，不用抄代码 |
| chalk / Bun.color | 我们现有 `theme.rs` 的 ANSI 生成 | 已有 |
| `Component.render(width) -> string[]` | 我们现有 `Line` 列表渲染 | 已有 |

### 9.2 性能损耗
omp 的重是 **TypeScript 运行时 + 组件树全量求值 + GC**，不是渲染算法本身。它真正聪明的三件事（append-only 块协议、冻结前缀 lex、行级 diff）我们**已经用不同方式实现了等价或更好的版本**（BlockCache + 自底向上视口 + 高度 sticky）。移植后的增量成本：
- pulldown-cmark 每帧只 lex 尾巴：O(增量)；
- syntect 高亮走 LRU + ParseState 流式：只高亮新完成的行；
- 上屏：ratatui diff 我们已有。
预期单帧成本与现在同数量级，**内存有界不变**。

### 9.3 需要新增的依赖总账
`pulldown-cmark`（唯一新增）。syntect、unicode-width 已有。无 C 库（regex-fancy 纯 Rust）。

## 十、建议的移植切片（拍板用）

按"渲染效果提升/工作量"排序：

1. **Markdown 全面升级**（最大收益）：pulldown-cmark 替换自写 markdown.rs——列表（有序/无序/嵌套/悬挂缩进）、h1 下划线、引用竖线、代码块边框行灰色。抄 omp 的渲染规则表（第二节），值抄 dark.json 的 md* 色。
2. **代码块 syntect 语义高亮**：抄 scope→11 类映射，块完成后整体高亮 + LRU；流式中只高亮已完成行。
3. **工具卡升级**：标题行 `icon title: desc [badge] meta` 格式 + 状态符号表 + running 时 spinner 帧入标题；framed 圆角框我们已有（4e6f2e0）。
4. **thinking 脉冲**：8 帧花型 + 余弦缓动 + toks/s 徽标。工作量小，观感收益大。
5. **用量行**：turn 结束 dim 色一行 metrics（我们已有 usage 数据，只差渲染）。
6. **用户气泡**：已有黑底，补"气泡内 markdown 化"（现在纯文本）。
7. **不做**：session accent 公式、themeEpoch 热切换、OSC8/OSC133、mermaid/latex、textSizing 双倍高字符。

关键取舍说明：我们**不抄** omp 的组件树全量求值+diff 架构（它受限于 TS 运行时才这么设计），保留 BlockCache 自底向上方案——omp 自己的 issue (#2081) 证明全量路径是它的性能痛点，我们没有理由倒退。
