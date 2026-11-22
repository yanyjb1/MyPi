# webtest — Web of Science 读取流程的验收单

一次验收分三步：打开检索页 → 检索结果页 → 点进详情页。每步只做一件事：用 `web` 工具读页面，
然后照着下面的清单核对输出。核对的是**输出里有什么、没有什么**，不是页面在浏览器里长什么样。

## 前提

1. 规则集改完必须**重启 mypi** 才生效；进程启动时只读一次规则。
2. 源码 `src/server/config/web-rulesets/wos.json` 与 `~/.config/mypi/web-rulesets/wos.json` 必须一致。
3. 浏览器是有头的，登录态存在 profile 里。若弹出验证页，等人点过去，工具会自己等。

## 第一步：检索页

`web` `open` → `https://www.webofscience.com/wos/woscc/basic-search`，等 6–8 秒再 `read`。

核对：

- 正文里出现 `<button>Web of Science Core Collection</button>`、`<button>All</button>`、
  `<button>Topic</button>`、`<button>Add row</button>`、`<button>Add date range</button>`、
  `<button>Clear</button>`、`<button>Search</button>`。
- 这些按钮**不得**带任何属性，例如 `class=`、`mat-stroked-button=`、`_ngcontent-`。
- 搜索框在正文里是空的 `<input>`，不是 `<input type="text" ...>`。
- 清空行的图标按钮在正文里是 `<button></button>`（空标签），不是消失。
- 正文里**不得**出现 `clear`、`search`、`arrow_drop_down` 这类图标字。
- `operable` 里应当有 `input:text "Search box 1 Topic, ..."` 与 `button "search Search"`，
  选择器可点。

## 第二步：检索结果页

`web` `fill` 选择器 `#search-option-0`，值：

```
("large language model" OR "generative artificial intelligence") AND ("public opinion" OR "sentiment analysis")
```

`web` `click` 选择器 `#snSearchType > div:nth-of-type(3) > button:nth-of-type(2)`，等 12–15 秒再 `read`。

核对：

- 首行是 `# 591 results from Web of Science Core Collection`（数字可能随数据库更新变化）。
- 正文里出现 Refine 面板的分区名：Publication Years、Document Types、Researcher Profiles、
  Web of Science Categories、Affiliations、Publication Titles、Languages、Countries/Regions、
  Publishers、Research Areas、Open Access、Funding Agencies ……
- 正文里出现每条记录的标题链接 `[...](/wos/woscc/full-record/WOS:...)`、作者链接、被引数与参考文献数。
- 正文里**不得**出现：`Copy query link`、`Create Alert`、`Export Refine`、
  每条记录的「更多操作」（`Options for record`）、`Enriched Cited References`。
- 正文里**不得**出现任何 `api/gateway` 开头的 GetFTR 链接。
- 正文里**不得**出现图标字：`add`、`expand_more`、`chevron_right`、`keyboard_arrow_left`、
  `keyboard_arrow_right`、`arrow_drop_down`。
- 勾选框在正文里是 `<input>`，后面跟着它的文字，例如 `<input>` 换行 `Review Article 26`。
- `operable` 的**前十行必须是真控件**（button / input / select / textarea / summary / 真链接），
  不能是面包屑那种 `span`、`div` 文字块。面包屑可以出现在后面，但不得占前十。
- `operable` 的名字里**不得**残留图标字，例如 `button "Export expand_more"` 是错误。

## 第三步：详情页

从结果页点第一条记录的标题（`web` `click` 选择器 `app-record h3 a`），等 8–10 秒再 `read`。

核对：

- 正文**开头就是控件**：`<button>Full Text Links</button>`、`<button>Export</button>`、
  `<button>Add To Marked List</button>`，之后才到 `##` 标题。
- 正文里**不得**出现 `api/gateway` 链接，也不得出现 `[Full text at publisher![...]]` 这种图片包链接。
- 标题、`### By` 下的作者（带全名与机构编号）、`### Edited by`、`### Source`、`### DOI`、
  `### Page`、`### Document Type` 齐全。
- `## Abstract` 是完整段落，不是被截断的半句。
- `### Author Keywords` 与 `### Keywords Plus` 齐全。
- `## Funding` 的表格在，基金号（如 `PID2022-139297OB-I00`）在。
- `## Citation Network` 在，被引数、`### Cited References` 数字在。
- 正文里**不得**出现：`Are you this author?`、`affiliation details`、`View funding text`、
  `Show All Details`、`Show details`、`See more data fields`、`Create citation alert`、
  `See more times cited`、`View Related Records`、`Open comparison metrics panel`、`Learn more`。
- 正文里**不得**出现作者名后的机构编号链接 `[1]`。
- `operable` 里前十行是真控件，不得是 `span "Trillo, JR"` 这类文字碎片。
- `operable` 的名字里不得残留图标字。

## 判定

三步全部对上，这一轮通过。任何一步不符，把不符的那一条和它所在的步骤报出来，不要自己改代码。