# web

A self-contained domain for reading pages, searching, and driving a browser.
The tool layer maps a call's arguments into `WebArgs` and hands it over: `web` for the work tab, `search` for searches.
Everything below that line — Chromium, CDP, tabs, providers, HTML conversion — belongs here and nowhere else.
It is a cargo feature, on by default: a build without it has no `web`, no `search` and no site rules.
It is not part of the tool layer, and it decides for itself when the browser starts and when it dies.

## 1. Design / Schema

### One operation

`WebArgs` is the only input; `op` selects the operation and all other fields are shared.

| op | input | tab | output |
|---|---|---|---|
| `read` | `url`, `raw` | work | Markdown, or the DOM written to a file |
| `search` | `query`, `provider`, `pages` | search | provider name, page range, result count, numbered hits |
| `open` | `url` | work | one status line |
| `click` | `selector` | work | one status line |
| `fill` | `selector`, `value` | work | one status line |
| `select` | `selector`, `value` | work | one status line |
| `scroll` | `value` (pixels, default 600) | work | one status line |
| `eval` | `value` (JS expression) | work | evaluation result |
| `screenshot` | `full_page` | work | file path and byte count |

`run(args, session)` is the whole entry: it takes one of two permits, dispatches on `op`, and returns the text the model reads.

### Browser process

One Chromium per process, always headed, launched on the first operation and never before; `config.server.agent.tools.browser.bin` must name the binary before that launch.

```text
ensure
  → a live handle in STATE?                        → reuse it
     (an owned child that exited or a dead port is replaced)
  → MYPI_BROWSER_PORT, config port                 → attach
  → DevToolsActivePort of the profile              → attach
  → otherwise                                      → launch
```

- `owned` browsers run in a container of their own and closing stops the whole container, while `attached` browsers belong to the user and are never signalled or idled out.
- The idle clock is set at launch and at the end of every tab lease; ten minutes without an operation closes an owned browser, tabs and group and all.
- `shutdown()` closes the owner and forgets every tab, idempotently; `main` calls it after the IPC server returns, and a signal thread calls it for INT, TERM and HUP.
- An owned browser must not survive the server, so the platform binds every following tree to the main process.
- No display is an error, not a fallback: there is no headless mode.

Launch argv:

```text
<bin>
  --user-data-dir=<profile>
  --remote-debugging-port=0
  --remote-debugging-address=127.0.0.1
  --no-first-run
  --no-default-browser-check
  --disable-blink-features=AutomationControlled
  <config.server.agent.tools.browser.extraArgs…>
  about:blank
```

The profile is persistent — logins, cookies and user extensions live there — and the User-Agent is left alone because a headed Chromium already reports `Chrome/<major>` consistently with `Sec-CH-UA`.

### Search

Three engines, one per call — `google`, `bing`, `baidu` — and the loop walks pages 1..=`pages`, stopping at the first page that adds nothing new, with a 1.5–3 s pause between pages.

```text
google  https://www.google.com/search?q=…&udm=14&pws=0&start=10×(n-1)
bing    https://www.bing.com/search?q=…&first=10×(n-1)+1
baidu   https://www.baidu.com/s?wd=…&pn=10×(n-1)
```

- A result's `href` is not always a destination: Google mints `/goto?url=<token>`, Baidu mints `/link?url=<token>`, and both only resolve through the engine's own answer of a `302` with `Location` or a `200` holding a meta refresh; `provider::settle` asks once per hit and drops what does not resolve to an external page, while Bing's `a1<base64url>` is decoded locally.
- An external host is not enough: ad gateways and the engines' own search paths are not destinations, but content hosts on an engine's own domain are kept.
- Hiding is not removing: `DOM.getOuterHTML` serializes the DOM rather than what is drawn, so before parsing the search tab deletes every result container with no layout box and re-reads the document.
- A gate is not diagnosed: `normal()` only answers whether the DOM is the engine's own results page, and anything else is something a human has to clear.

### Tabs

`tabs::POOL` is the process's only tab state.

```text
(session, TabKind::Search)  → search
(session, TabKind::Work)    → read, open, the actions, screenshot
```

- Tabs are created on demand and keyed by session and kind, and a tab that vanished from the browser is recreated on the next call.
- Every tab is serial per session through a slot lock held for the whole call, so two calls in one session queue rather than interleave, and the pool lock only guards the maps.
- A tab closes after 30 minutes without an active call, and `active` counts the call in flight so a busy tab is never reclaimed underneath itself.
- `close_session` closes both of a session's tabs and `unbook` calls it, so a session that ends takes its tabs with it.
- The `about:blank` the launch argv opens is reclaimed once a pool tab exists, so an owned browser shows the session's tabs and nothing else.

### Verification

A gate is not diagnosed: the provider only answers whether the DOM is its own results page, and the domain waits for a human instead of failing.

1. If `config.web.bringToFront` is true the tab is raised once; the flag only decides whether the window comes forward, and polling happens either way.
2. Every 30 seconds the current URL and DOM are handed to a predicate supplied by the caller, which by default is the provider's `normal()`.
3. Passing inside five minutes is silent: the page is fetched again and the operation continues as if nothing happened.
4. Five minutes without passing returns an error saying the user may be away, and a later retry starts over.

The poll never navigates, so the page the user is working on is not reset.

### The permit

At most two operations run at once across the process, and the third waits on a condition variable; the permit is taken once in `run`, so no operation can forget it, and the idle sweep reads the same count.

### Output

`read` answers Markdown, or with `raw` writes the DOM to `<temp>/mypi/web/<site>-<n>.html` and answers that path; `search` answers the provider, the page range, the count and a numbered list; the rest answer a status line, except `screenshot`, which names its file and its size.

Every file under `<temp>/mypi/web` is named for the site it came from and numbered in the order it was written, so two calls in the same second never land on each other.

### What a read leaves out, and what it adds

The answer is the page's prose, not its document: a read that names a URL navigates first and polls until the `<body>` is parsed and the text is not a script shell, and a page still unfinished when the budget ends is read as it stands. A body that scans to nothing is given three seconds and scanned once more before the answer calls it empty. Blank lines are collapsed to one, so a gap in the prose is never a wall of them.

A session keeps the prose of its last read. A read of the same address answers a header counting the lines gone and the lines added, then the added lines alone; a different address, and the first read of one, answers the whole page. The record lives in memory and ends with the session or the process.

- **SVG goes, whatever holds it.** Every `svg` element is cut out of the markup before conversion, so a button keeping its icon loses the icon and stays a button.
- **Inline images go.** An `img` whose `src` is a `data:` URI is dropped and its alt text is kept, because a marketing page's icons routinely outnumber the prose; an image naming a real address is left as it is.
- **The head's metadata goes.** No frontmatter is emitted at all, because a page's `<meta>` tags describe crawlers and the site's own scripts rather than the article.
- **Landmarks go.** `<nav>`, `<footer>`, `<aside>` and `<form>` are removed, along with the elements whose class or role names them as chrome.
- **Controls stay, reduced.** Every `button`, `input`, `select`, `textarea` and `summary` in the prose becomes one `<button>` around the text it shows, and a control showing no text leaves nothing behind.

The `operable` section names each control's selector, because `click` and `fill` take one and Markdown has nowhere to put it; up to ten are listed inline, and a page offering more has every one of them written to a file the section points at.

- **A control is named once.** Widgets nest — a checkbox is an `input` inside four decoration layers, a button wraps its own label — so a chain answers once: a widget beats the shells around it, a bare shell answers by the innermost one, and a shell whose subtree holds a widget leaves the entry to that widget.
- **What is named is what an action addresses.** A widget inside a shell is named, never the shell, even though the shell's path is shorter.

A page whose body renders no text at all answers `target url has an empty body` rather than an empty document, so the model can tell an unrendered page from a quiet one.

### Site rules

A site whose `<body>` mixes prose with furniture needs naming, and the naming lives in `<config dir>/web-rules.json` beside `config.yaml`, read once when the configuration is loaded.

A missing file is given the built-in one on first launch, sets and all, and a file that cannot be read is named in a notice while the built-in set answers instead.

The file is a manifest: one array whose entries are either a rule written in place or a string naming another file of rules, each set flattened in at the position it is named, and the top-level order is the order rules are tried in.

```json
[
  "web-rulesets/court.json",
  { "match": "^example\\.com/doc/", "keep": [{ "selector": "main", "drop": ["nav"] }] },
  "web-rulesets/thepaper.json"
]
```

A set's path is relative to the manifest's own directory and written with forward slashes, and a set that cannot be read is one warning and one hole while the rest of the manifest still applies.

The built-in set is authored the same way — the manifest names one set per site under `server/config/web-rulesets/`, and every one is compiled into the binary — so the files the user edits are the files that were read.

Within a manifest or a set, rules are tried in order, and the first rule whose `match` — a regex over `host + path`, never the query — matches the page is the rule that applies.

```json
{
  "match": "^example\\.com/doc/",
  "keep": [
    { "selector": "main", "drop": ["nav", "footer"] },
    { "selector": "#appendix", "drop": [], "plaintify": ["aside"] }
  ]
}
```

- `keep` is the prose: each entry names one region by a selector looked for from the body, the first element it names is what is read, and several entries are read in order and concatenated.
- `drop` applies inside its own entry only: the named elements are ignored — never read, never offered as controls — and no element beside them moves.
- `plaintify` applies inside its own entry only, to the whole subtree: an element it names is read as its words with block boundaries kept as line breaks, and nothing inside it is offered as a control or names itself by an attribute.
- When no rule matches, or none of a rule's selectors resolve, the page is read whole, because a broken rule must not hide a page.

The `operable` section obeys the same regions: a narrowed read scans the kept regions only, an entry's `drop` and everything under its `plaintify` are left out of the control list exactly as they were left out of the prose, and a control's name loses what a drop removed.

A site that writes no `<main>` gets no special handling; the regions above are how such a site is named, and until it is, its whole body is what is read.

## 2. Design intent

- **A domain, not a tool.** The browser's behaviour is its own — when to start, how many tabs, when to reclaim them, when to die — and the tool layer gives the model a convenient verb without steering it.
- **Always headed.** A challenge needs a human in front of a window, and a headed browser carries fewer automation tells than any headless mode.
- **One browser per process.** One persistent profile keeps logins, one window keeps the user able to supervise, and one tree gives the exit path a single handle.
- **Owned versus attached.** A browser the user started is theirs, and one the domain started is a child of the server that must not survive it.
- **The platform owns the binding.** `PR_SET_PDEATHSIG` on unix and the job object on windows both bind the tree, so a connection thread's end cannot orphan the browser.
- **Idle close.** The profile is on disk, so closing costs only a relaunch while leaving the process up costs an orphan every time the server goes away badly.
- **Two tabs per session.** A search tab may be replaced freely and a work tab must survive between `open`, `click`, `fill` and `read`, and a session that ends closes both.
- **A serial tab.** One call at a time per tab keeps a gate attributable to the engine that raised it.
- **A silent verification.** The model asked for results, and a human solving a challenge is infrastructure rather than an outcome, so only a timeout is worth reporting.
- **30 seconds and five minutes are constants.** They are interaction pacing, and a config key would invite tuning that makes the wait either useless or hostile.
- **Config read late and once.** Reading at launch keeps a long-lived session from re-parsing the file on every call.

## 3. Unresolved

- macOS has no parent-death equivalent, so only the explicit exit path covers it there.
- Attached mode cannot switch a user's browser between headed and headless, which is irrelevant now since there is no headless mode.
- The parsers are tuned against captured live pages, and Baidu's visible ads are not separated from organic results.
- A machine without a display cannot use this domain at all.

## 4. Config fields

| key | use |
|---|---|
| `config.server.agent.tools.browser.bin` | the executable, required before the browser tool can launch one |
| `config.server.agent.tools.browser.profileDir` | the persistent profile; empty = `<data dir>/browser/profile` |
| `config.server.agent.tools.browser.port` | attach to a running browser instead of launching |
| `config.server.agent.tools.browser.idleSecs` | seconds of no activity before an owned browser is closed; default 600 |
| `config.server.agent.tools.browser.userAgent` | override the launched browser's User-Agent; unset = the browser's own |
| `config.server.agent.tools.browser.extraArgs` | extra argv |
| `config.web.bringToFront` | raise the window when a verification challenge appears; default true |