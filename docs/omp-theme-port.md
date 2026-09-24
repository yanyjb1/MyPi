# omp 主题系统移植方案（抄全套色值系统）

> 需求回顾：用户要 omp 的色值系统整套——主题 JSON、token 体系、运行时切换（如重命名会话时触发换色）、启动时读配置默认主题、运行中替换是临时变量不落盘。不要 omp 的花哨周边（OSC11 明暗自动探测、watcher 热重载这些可以裁）。

## 一、omp 的主题系统全貌（源码级）

文件与规模：`theme.ts`(810, 全局态+切换) / `theme-class.ts`(787, Theme 类) / `loader.ts`(200, 发现+合并) / `color.ts`(95, 颜色数学) / `schema.ts`(168, token 枚举+校验)。

### 1.1 数据模型（三层）

```
ThemeJson {
  name, $schema,
  vars:    { "electricBlue": "#00b4ff", ... }        // 语义变量，可互相引用
  colors:  { "accent": "electricBlue", ... }         // token -> var名 或 直接hex 或 256索引
  symbols: { preset: "ascii", overrides: {...}, spinnerFrames: {...} }
  export:  { pageBg, cardBg, infoBg }                // 给 web 端的，TUI 不用
}
```

- **vars 引用解析**：`resolveVarRefs` 递归解引用，环引用报错（color.ts:48-63）。值三种形态：`""`（终端默认色）、`"#hex"`、`number`（256 色索引）。
- **fg/bg 分类**：7 个固定 Bg token（selectedBg/userMessageBg/customMessageBg/toolPendingBg/toolSuccessBg/toolErrorBg/statusLineBg），其余全是 fg token（loader.ts:153-167）。
- ThemeColor 全集 **62 个 token**（schema.ts:26-86），ThemeBg 7 个。核心子集见下节。

### 1.2 Theme 类 = 一次性预编译缓存

构造时把每个 token **预编译成两种形态**（theme-class.ts:171-186）：
- `#fgColors[key]` = ANSI 序列（truecolor `\x1b[38;2;r;g;b m` 或 256 色 `\x1b[38;5;Nm`）
- `#hexFgColors[key]` = CSS hex（给 accent 数学、状态栏插值用）

运行时 API 全是查表拼串，**零颜色计算**：

```ts
fg(color, text)   => `${ansi}${text}\x1b[39m`    // 只重置前景，不动背景
bg(color, text)   => `${ansi}${text}\x1b[49m`    // 只重置背景
bgFill(color, t)  => 嵌套 \x1b[0m 后续接背景（防重置穿透）
fgOnBg(fg, bg, t) => 底色亮度 > 0.5 用黑字否则白字
```

关键细节：**只重置前景 `\x1b[39m` / 只重置背景 `\x1b[49m`**，不整段 reset——这是 omp 卡片底色不花的原因。`text: ""` 表示终端默认色时 ANSI 序列就是裸 reset。

### 1.3 主题发现与合并（loader.ts）

1. 内置主题 = dark.json + light.json + defaults/*.json（**101 个**，编译进二进制）；
2. 自定义主题目录 `~/.config/omp/themes/*.json`，文件名即主题名，与内置同名时**内置优先**（builtinThemes 先查）；
3. 主题 JSON 先过 schema 校验（缺 token 报错并列出缺失清单）；
4. `symbols.preset`：settings 覆盖 > 主题 JSON > 默认 unicode（loader.ts:177）。
5. 失败一律回退 dark + notify。

### 1.4 运行时切换（theme.ts 全局态）

```ts
export var theme: Theme;          // 全局单例
let themeEpoch = 0;               // 单调计数器
setTheme(name)      -> loadTheme -> assignTheme -> themeEpoch++ -> onThemeChange 回调
previewTheme(name)  -> 同上但 event.ephemeral=true（预览不重建 scrollback）
setThemeInstance(t) -> 内存实例直接换（我们的"临时变量"场景就是这个）
```

消费端失效模式（interactive-mode.ts:1730-1747）：

```
onThemeChange -> 清各处缓存 -> ui.invalidate() -> requestRender()
  ephemeral（预览）: 只重画 viewport，不碰已进 scrollback 的历史
  正式切换       : clearScrollback + 全量重绘（滚上去的旧行也换色）
```

**epoch 进缓存键**：tool-execution.ts:800 的缓存 key 里拼了 `getThemeEpoch()`；statusline 缓存同样。任何依赖颜色的记忆化都必须把 epoch 算进键，否则换主题后出旧色。

### 1.5 色彩数学（color.ts + pi-utils/color）

- `colorLuma`（Rec.601 感知亮度）：明暗判定、fgOnBg 对比字色（>0.5 黑字否则白字，theme-class.ts:377-378）。
- `relativeLuminance`（WCAG）：light 主题 accent 对比度预算用。
- 256 索引 ↔ hex 互转（6x6x6 cube + 灰阶 ramp）。
- truecolor 探测：`COLORTERM` / 终端特征表（terminal-capabilities.ts:657），探测失败降级 256color。

## 二、MyPi 移植方案

### 2.1 新增 `tui/theme/` 目录（替换现 theme.rs 平铺）

```
tui/theme/
  mod.rs      // 全局 THEME 单例 + epoch + init/set_theme
  token.rs    // ThemeColor/ThemeBg 枚举（62+7）+ 强类型查表
  class.rs    // Theme 结构：预编译 ANSI+hex 双缓存、fg()/bg()/bg_fill()/fg_on_bg()
  loader.rs   // 内置 JSON（include_str!）+ ~/.config/mypi/themes/*.json 发现
  color.rs    // hex<->rgb、luma、256索引互转、truecolor 探测
```

依赖：serde_json（已有）。不新增任何 crate——颜色数学手写 luma/相对亮度就是十行。

### 2.2 token 集：裁剪到聊天区必需

omp 62 个 token 里含 statusLine* 14 个（状态栏用，保留）、thinkingOff..Max 7 档（保留）、bashMode/pythonMode（我们没有 bash/python 特化模式，**裁掉**）。裁剪后约 50 个，一次定义枚举，JSON 缺 token 启动时报错列出缺失（抄它的报错格式，很好用）。

### 2.3 运行时切换（照用户的 trigger 设计）

```rust
// mod.rs
static THEME: RwLock<Theme>        // parking_lot，切换只在主线程读，无争用
static THEME_EPOCH: AtomicU64

pub fn set_theme_by_name(name: &str) -> Result<()>   // 运行中切换（不落盘）
pub fn theme_epoch() -> u64                           // 缓存键成分
```

- **启动链**：run_tui 开头 `init_theme()`——读 config.yaml 的 `theme.dark: titanium`，加载失败回退内置 dark（我们把 titanium 作为第一内置主题打包，dark.json 做兜底）。
- **trigger 落点**：`/name` 命令处理完 → `set_theme_by_name(...)`（用户提的"重命名会话时换主题"就是在这个点插一行）。omp 的 previewTheme 我们先不做 UI 预览，但 ephemeral 标志保留在 API 签名里。
- **失效传播**：epoch 塞进 BlockCache 的 Slot 键和 statusline 渲染缓存键；切换时 `block_cache.clear()`（宽变清全的现成路径复用）。
- **历史区换色**：omp 正式切换会 clearScrollback 重绘。我们的 transcript 本来就是"内存账本按需物化"——重绘天然换色，**不需要**清终端 scrollback，这比 omp 干净。唯一要做的是切换后强制重画一帧。

### 2.4 与符号系统合流

symbols（ascii 预设表，见 omp-symbols-ascii.md）作为 ThemeJson 的 `symbols` 段一起进 JSON：preset 选择 + per-key overrides + spinnerFrames 覆盖，校验后编译进 Theme。这样一个主题文件 = 色 + 符号 + 帧完整套装。

### 2.5 JSON 主题格式（与 omp 完全兼容）

直接用 omp 的格式（vars + colors + symbols），这样 101 个内置主题里看上哪个抄哪个，甚至 omp 社区主题文件可以原样放进 `~/.config/mypi/themes/`：

```json
{
  "name": "titanium",
  "vars": { "electricBlue": "#00b4ff", "darkTitanium": "#0f1216", ... },
  "colors": { "accent": "electricBlue", "userMessageBg": "darkTitanium", ... },
  "symbols": { "preset": "ascii", "overrides": {} }
}
```

### 2.6 色值包（首发两个内置主题）

1. **titanium**（用户当前在用的）：完整色值已在 omp-symbols-ascii.md §七，直接落 JSON。
2. **dark**（omp 兜底主题）：vars/主色已扒（accent 金 #febc38、green #89d281、userMsgBg #221d1a 等），作回退。

用户说"绿色套装换蓝换红"——本质就是换 accent/syntaxX 一组值。可以后续给 titanium 做 accent-only 变体（vars 只改 electricBlue），101 个主题随取随抄，格式零转换。

## 三、工作量评估

| 项 | 内容 | 量 |
|---|---|---|
| color.rs | hex/rgb/luma/256 互转/探测 | ~120 行，纯函数好测 |
| token.rs | 枚举 + JSON 键映射 | ~150 行，机械 |
| class.rs | 预编译 + fg/bg/bg_fill/fg_on_bg | ~200 行 |
| loader.rs | include_str! 内置 + 目录发现 + 校验 | ~150 行 |
| mod.rs | 单例 + epoch + 切换 API | ~80 行 |
| 接线 | view/components/transcript 的着色点改走 theme token（现 Palette → Theme 平移） | 主要是机械替换，~半天 |
| 主题 JSON | titanium + dark 两个 | 数据 |
| 切换触发 | /name hook + config 读取 | ~30 行 |

总计约 700-800 行新代码 + 一轮着色点替换。无新依赖（serde_json 已有）。
性能影响：零——所有色值预编译查表，和现在 Palette 一样；epoch 只进缓存键不进热路径。

## 四、明确不抄的（omp 里有但我们裁掉）

- OSC 11 终端底色探测 + dark/light 自动切换（SIGWINCH/Mode 2031 一整套）——单终端场景用不上。
- 主题文件 watcher 热重载。
- colorBlindMode（HSV 移相）。
- session accent 推导公式（djb2+OKLCH 那套，太重，且我们不需要 per-session 色）。
- mermaid/latex 相关 token。
