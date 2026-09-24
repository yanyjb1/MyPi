# omp ASCII 符号对照表（用户配置：symbolPreset=ascii + titanium 主题 + rounded-box 状态栏）

> 来源：`/tmp/oh-my-pi/packages/tui/src/theme/symbols.ts`。omp 有三套符号预设 unicode/nerd/ascii，主题 JSON 里 `symbols.preset` 决定用哪套（loader.ts:179）。用户当前配置是 **ascii**（`~/.omp/agent/config.yml` 首行），主题 **titanium**，状态栏 rounded-box。
> 全部 ASCII 预设符号**零 emoji 零 Unicode 花体**——这也回答了"死人的 emoji 从哪来"：那是 unicode/nerd 预设的。抄作业照 ascii 抄，全程无 emoji。

## 一、Spinner 帧（80ms/帧）

| 类型 | ascii 帧 | unicode 对照 |
|---|---|---|
| **status**（工具卡运行态） | `\| / - \` | 盲文 `⣾⣽⣻⢿⡿⣟⣯⣷` |
| **activity**（思考/品牌/状态栏时钟） | `- \| / \` | 盲文 `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`（10帧） |

- 定义：symbols.ts:1385-1401。所有 live 工具块共享一个 ticker 同相位（tool-execution.ts:184-234）。
- 我们现状 `|/-\` 与 ascii status 帧一致，可以直接对齐成两套（status 4帧 + activity 4帧、顺序修正为 `-\|/`）。

## 二、状态符号（工具卡标题、状态行）

| key | ascii | unicode（你讨厌的那套） | 着色 |
|---|---|---|---|
| success | `[ok]` | ✔ | success |
| error | `[!!]` | ✘ | error |
| warning | `[!]` | ⚠ | warning |
| info | `[i]` | ⓘ | accent |
| pending | `[*]` | ⏳ | muted |
| running | `[~]`（静态）| ⟳ | accent |
| aborted | `[-]` | ⏹ | error |
| done | `*` | • | success |

运行中的卡标题 icon = **spinner 当前帧**（render-utils.ts:290-295），即运行态显示 `\|/-` 旋转而非 `[~]`。

## 三、工具身份 glyph（成功态标题前缀，symbols.ts:1310-1360）

| 工具 | ascii | unicode |
|---|---|---|
| bash | `$` | ❯ |
| edit | `~` | ✎ |
| write | `+f` | ✎ |
| browser | `[w]` | 🌐 |
| web search | `web` | ⌕ |
| task | `>>>` | ⇶ |
| todo | `[x]` | ☑ |
| read / fetch | （无专属 glyph，标题用状态符号 pending/success/error） | |

read/search/fetch 这类"查询型"工具在 omp 里不配身份 glyph（read.ts:286 直接 `icon: pending`），只有 bash/edit/browser 这种"动作型"才有。**这是个值得抄的取舍**：少即是多。

## 四、Markdown 符号（ascii 预设）

| key | ascii | unicode | 用途 |
|---|---|---|---|
| md.bullet | `*` | • | 列表前缀（ordered 用 `N.`） |
| md.quoteBorder | `\|` | ▏ | 引用块行前缀 |
| md.hrChar | `-` | ─ | 水平分隔线 |
| format.bracketLeft/Right | `[` `]` | ⟦ ⟧ | badge 括号 |

代码块边框行 ``` 本身不换符号，只换色（mdCodeBlockBorder）。

## 五、Box 字符（rounded-box 在 ascii 下退化为 +）

| 位置 | ascii | unicode |
|---|---|---|
| 四角 | `+` | ╭ ╮ ╰ ╯ |
| 横线 | `-` | ─ |
| 竖线 | `\|` | │ |
| tee（分节分隔线左右端） | `+` | ├ ┤ |

注意：**ascii 预设下"rounded box"和 sharp box 完全同形**（都是 `+ - |`）。你看到的"rounded 状态栏"效果实际来自钛主题的 border 色 + 布局，字符层全是 ASCII。工具卡分节分隔线 `+── Output ──+`。

## 六、其它高频符号

| key | ascii | unicode |
|---|---|---|
| nav.cursor（输入区提示） | `>` | ❯ |
| inputCursor（编辑器光标字符） | `\|` | ▏ |
| checkbox checked/unchecked | `[x]` / `[ ]` | ☑ / ☐ |
| icon.tokens | `tok:` | 🪙 |
| icon.context | `ctx:` | ◫ |
| icon.cost | `$` | 💲 |
| icon.time | `t:` | ⏱ |
| icon.omp（品牌） | `pi` | π |
| sep.dot（标题 meta 分隔） | ` - `（三字符，含空格） | ` · ` |
| format.bullet | `*` | • |

## 七、titanium 主题色值（vars 解析后，抄色用）

```
accent        #00b4ff（电蓝——标题、运行态边框、md标题/列表圆点/链接）
text          (默认前景，未覆写)
muted         #9ca3af(dimAluminum)   描述、思考正文（斜体）
dim           #6b7280                 meta、done 符号、成功卡边框
border        #2a3038  borderMuted #1f252d
success       #00ff88(readoutGreen)   error #ff4757   warning #ffb347
thinkingText  #9ca3af   thinkingHigh #00b4ff
mdCode        #00ff88   mdCodeBlock #9ca3af   mdCodeBlockBorder #2a3038
mdQuote #9ca3af  mdQuoteBorder #2a3038  mdHr #2a3038  mdLinkUrl #0082b3
syntaxComment #6b7280  keyword #00b4ff  string #d4c090  function #00ff88
syntaxNumber #ffb347  type #00b4ff  variable #e8ecf4  operator #00b4ff  punctuation #9ca3af
toolPendingBg #0f1216  toolSuccessBg #0f1216  toolErrorBg #1a0f10
userMessageBg #0f1216  statusLineBg #0f1216
```

注意钛主题的成功卡边框走 `dim`（灰），错误才用 error 红——边框色规则见 survey 报告 §四。

## 八、工具卡完整字符结构（ascii 实拍）

```
+── $ cat foo.txt ─────────────+     <- 顶栏: boxRound.topLeft + label(icon+title) + horizontal + topRight
| $ cat foo.txt | head -5      |     <- 命令段（无标签）
+── Output ────────────────────+     <- 分节栏: teeRight + 标签 + teeLeft
| hello world                  |
| [Wall: 0.42s | Exit: 0]      |     <- stats 行: dim 色 + bracketLeft/Right
+──────────────────────────────+     <- 底栏: bottomLeft + horizontal + bottomRight
```

- 顶栏 label = `{icon} {title}: {desc} [badge] meta`，label 与横线之间各空 1 格（output-block.ts:170 ` ${labelText} `）。
- 运行态：边框 accent 色 + 顶栏 icon 是旋转的 `\|/-`。
- 内容行：`| ` + 内容 + 右填充到宽 + ` |`；内容超宽先 ANSI 感知折行（wrapTextWithAnsi）。
- 宽度不足画边框时自动放弃边框（clipFrame 逐行截断）。

## 九、给 MyPi 的落地建议（符号层）

1. 建 `symbols` 常量模块：上表 ascii 列逐项落成 `const`，全部来自 ascii 预设——**无 emoji**。
2. spinner 双轨：status `\|/-\`（工具卡）、activity `-\|/`（思考/状态栏），共享 80ms tick。
3. markdown 列表 bullet `*` / `N. `，引用 `| `，与现有黑底用户卡不冲突。
4. 工具卡标题行照 §八结构，先做 bash/fetch/browser/search 四卡。
5. emoji 禁令从符号层自然满足：ascii 预设里一个 emoji 都没有。
