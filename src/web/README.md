# web

One tool-facing domain for reading pages, searching, and driving a browser. The tool layer maps arguments; this domain owns Chromium, CDP, tabs, providers, and HTML conversion.

## Schema

`WebArgs` is the only input. `op` selects the operation; all other fields are shared.

| op | input | tab | output |
|---|---|---|---|
| `read` | `url`, `raw` | search when `url` is set, work otherwise | Markdown or DOM HTML |
| `search` | `query`, `provider`, `page` | search | provider name, page range, result count, numbered hits |
| `open` | `url` | work | one status line |
| `click` | `selector` | work | one status line |
| `fill` | `selector`, `value` | work | one status line |
| `press` | `selector` (key name) | work | one status line |
| `select` | `selector`, `value` | work | one status line |
| `scroll` | `value` (pixels, default 600) | work | one status line |
| `eval` | `value` (JS expression) | work | evaluation result |
| `net` | `filter`, `max` | work | resource entries |
| `screenshot` | `full_page` | work | temp path and byte count |

Invariants:

- One process-wide Chromium; each session has one search tab and one work tab.
- The search tab is serial per session. The work tab is independent.
- A tab closes after 30 minutes without an active call. `reserve` protects the current work call.
- A search call uses exactly one provider and fetches pages `1..page` in order.
- Results are deduplicated by URL; an empty page stops the page loop.
- Search never falls back to plain HTTP. Every provider is read through the browser.
- CAPTCHA handling is a two-step handoff: first detection returns a retry hint; retrying the same provider opens a headed browser.
- Screenshots always go under the system temp directory.
- `net` reads `performance` resource entries only; it does not use CDP Network.
- At most two `web` operations run at once; the third waits.

## Design intent

The flat `op` schema keeps one verb per call. There is no nested action field, so the model never has to combine two operation names.

`src/web` owns everything that talks to Chromium. The tool entry only copies fields into `WebArgs`; it does not know URLs, selectors, providers, or files. This keeps browser failures and provider quirks out of the tool layer.

Each session has two named tabs because search and interactive work have different lifetimes. Search can be replaced freely; the work tab must survive between `open`, `click`, `fill`, and `read`. A 30-minute idle timer keeps tabs from accumulating without adding a freeze/unload state machine.

Search is single-provider because aggregation hides which index failed and makes retries ambiguous. The caller chooses the provider, and a CAPTCHA mark is scoped to that provider. Page fetching is sequential so a challenge on page N never returns a partial merged list.

The profile is always persistent. Logins, cookies, and user-installed extensions are the reason the browser exists; an ephemeral profile would make the tool less useful and add cleanup code. uBlock Origin is user-managed, not installed by this domain.

Screenshots use a fixed temp location so the model cannot write arbitrary files. The result carries the path for the user.

`net` is deliberately small: page performance entries answer "which endpoint did this page call" without a second event stream. Full request/response inspection belongs to a later network tool.

## 尚未解决

- Provider selectors are implemented but need live tuning against current result markup.
- A headed browser needs `DISPLAY` or `WAYLAND_DISPLAY`; attached browsers cannot switch to headed.
- CDP Network, HAR, and response bodies are intentionally absent.

## Config 字段

All keys live under `config.server.agent.tools.browser`:

- `bin` — Chromium-family executable.
- `port` — attach to an existing debugging port instead of launching.
- `profileDir` — persistent profile directory; defaults to `<data_dir>/browser/profile`.
- `headless` — launch headless. A headed browser stays headed for the process.
- `userAgent` — override the launched browser User-Agent.
- `extraArgs` — extra browser argv.
