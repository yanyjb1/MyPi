# webtest — Web of Science 读取流程

浏览器有头，登录态在 profile 里。

## 第一步

`web` `open` → `https://www.webofscience.com/wos/woscc/basic-search`
等 6–8 秒，`read`。

## 第二步

`web` `fill` 选择器 `#search-option-0`，值：

```
("large language model" OR "generative artificial intelligence") AND ("public opinion" OR "sentiment analysis")
```

`web` `click` 选择器 `#snSearchType > div:nth-of-type(3) > button:nth-of-type(2)`
等 3–6 秒，`read`。

## 第三步

`web` `click` 选择器 `app-record h3 a`
等 8–10 秒，`read`。