# web

The `web` tool entry: one call's arguments become `WebArgs`, and one call goes to the browser domain.
The browser, the tabs, the providers and the HTML conversion live in `src/browser`, and this tool is the model's verb.

## 1. Design / Schema

1. `type` — `read`, `open`, `click`, `fill`, `select`, `scroll`, `eval` or `screenshot`.
2. `url` — `read` and `open`: the page.
3. `raw` — `read`: write the DOM to a file instead of Markdown, `false` by default.
4. `selector` — `click`, `fill` and `select`: the element.
5. `value` — `fill` and `select`: the value; `scroll`: pixels; `eval`: the expression.
6. `pixels` — `scroll`: 600 by default.
7. `full_page` — `screenshot`: `false` by default.

`web` takes no array, and one call is one operation on the session's work tab.
Every operation is dispatched by the domain, so the browser's lifecycle, the tab pool, the search engines, the site rules and the output shapes are in `src/browser/README.md`.
The browser keys and the site rules are read by the configuration loader once and handed to the domain as a `Setup`; this tool passes it on and reads no file.

## 2. Design intent

- **A verb, not a browser.** The tool layer maps arguments, and the domain decides when a browser starts, how many tabs exist and when they are reclaimed.
- **The browser is optional at build time.** `web` is the cargo feature `browser`, on by default, because the browser stack is the largest dependency in the crate.
- **The work tab is separate.** `web` drives the session's work tab and `search` owns the search tab, so two operations never steer one page.

## 3. Unresolved

See `src/browser/README.md`, since the tool layer adds none.

## 4. Config fields

None at the tool layer, because the configuration loader reads the domain's keys and passes them as a value.