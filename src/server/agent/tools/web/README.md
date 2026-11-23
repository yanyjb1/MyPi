# web

The `web` tool entry: one call's arguments become `WebArgs`, and one call goes to the web domain.
The browser, the tabs, the providers and the HTML conversion live in `src/web`; this tool is the
model's verb.

## 1. Design / Schema

| field | value |
|---|---|
| `type` | `read`, `open`, `click`, `fill`, `select`, `scroll`, `eval`, `screenshot` |
| `url` | `read`/`open`: the page |
| `raw` | `read`: write the DOM to a file instead of Markdown, default `false` |
| `selector` | `click`/`fill`/`select`: the element |
| `value` | `fill`/`select`: the value; `scroll`: pixels; `eval`: the expression |
| `pixels` | `scroll`: default 600 |
| `full_page` | `screenshot`: default `false` |

`web` takes no array. One call is one operation on the session's work tab.

Every operation is dispatched by the domain; see `src/web/README.md` for the browser's lifecycle,
the tab pool, the search engines, the site rules and the output shapes.

The browser keys are read by the web domain itself, not by the tool layer.

## 2. Design intent

- **A verb, not a browser.** The tool layer maps arguments; the domain decides when a browser
  starts, how many tabs exist and when they are reclaimed.
- **The browser is optional at build time.** `web` is the cargo feature `web`, on by default, because the
  browser stack is the largest dependency in the crate.
- **The work tab is separate.** `web` drives the session's work tab; `search` owns the search tab,
  so two operations never steer one page.

## 3. Unresolved

See `src/web/README.md`; the tool layer adds none.

## 4. Config fields

None at the tool layer. The domain reads its own keys; see `src/web/README.md`.