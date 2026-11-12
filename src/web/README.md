# web

A self-contained domain for reading pages, searching, and driving a browser. The tool layer maps a
call's arguments into `WebArgs` and hands it over; everything below that line — Chromium, CDP, tabs,
providers, HTML conversion — belongs here and nowhere else.

It is not part of the tool layer. It decides for itself when the browser starts and when it dies,
which is why it reads its own configuration instead of receiving it through `Env`.

## 1. Design / Schema

### One operation

`WebArgs` is the only input. `op` selects the operation; all other fields are shared.

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

`run(args, session)` is the whole entry. It takes one of two permits, dispatches on `op`, and
returns the text the model reads.

### Browser process

One Chromium per process, **always headed**, launched on the first operation and never before.

```text
ensure
  → a live handle in STATE?                        → reuse it
     (an owned child that exited or a dead port is replaced)
  → MYPI_BROWSER_PORT, config port                 → attach
  → DevToolsActivePort of the profile              → attach
  → otherwise                                      → launch
```

- `owned` browsers are launched into a session of their own (`setsid`), so the group id is the
  leader's pid; closing signals the whole group TERM, waits two seconds, then KILLs and reaps.
- `attached` browsers belong to the user: this domain never signals them, and never idles them out.
- The idle clock is set at launch and at the end of every tab lease; an operation in flight also
  keeps the browser alive. Ten minutes without either closes an owned browser: tabs, group and all.
  The next operation launches a new one.
- `shutdown()` closes the owner and forgets every tab; it is idempotent. `main` calls it after the
  IPC server returns, and a signal thread calls it for INT, TERM and HUP before letting the signal
  take its default action.
- An owned browser must not survive the server. Linux gets that from `PR_SET_PDEATHSIG`, which is
  delivered when the *forking thread* ends; the launch therefore happens on one thread that lives
  as long as the process.
- No display (`DISPLAY` / `WAYLAND_DISPLAY` missing) is an error, not a fallback: there is no
  headless mode.

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

The profile is persistent: logins, cookies and user-installed extensions live there. The
User-Agent is not overridden; a headed Chromium already reports `Chrome/<major>` consistently with
`Sec-CH-UA`. `--disable-blink-features=AutomationControlled` is what keeps `navigator.webdriver`
false while a CDP client is attached.

### Search

Three engines, one per call: `google`, `bing`, `baidu`. Each page yields hits, and the loop walks
pages 1..=`pages`, stopping at the first page that adds nothing new. Pages are fetched one at a
time, with a 1.5–3 s pause between them.

```text
google  https://www.google.com/search?q=…&udm=14&pws=0&start=10×(n-1)
bing    https://www.bing.com/search?q=…&count=10&first=10×(n-1)+1
baidu   https://www.baidu.com/s?wd=…&pn=10×(n-1)
```

- A result's `href` is not always a destination. Google mints `/goto?url=<token>`, Baidu mints
  `/link?url=<token>`, and both only become the real URL through the engine's own answer: a `302`
  with `Location`, or — for some Google tokens — a `200` whose body is a meta refresh and one
  anchor. `provider::settle` asks once per hit — one at a time, 300–800 ms apart, six seconds each
  — and drops what does not resolve to an external page. Bing encodes the destination in
  `ck/a?…&u=a1<base64url>`, so it is decoded locally.
- An external host is not enough: ad gateways (`googleadservices`, `googlesyndication`, `ubs.baidu.com`,
  `bdstatic`, `bdurl`) and the engines' own search paths (`/search`, `/s`, `/ck`, …) are not
  destinations. Content hosts on an engine's own domain — `baike.baidu.com`,
  `developers.google.com` — are kept.
- Hiding is not removing. A card an ad blocker sets to `display: none` is still in the serialized
  DOM, and `DOM.getOuterHTML` serializes the DOM, not what is drawn. So before parsing, the search
  tab deletes every result container with no layout box — `display`, `visibility`, or a hidden
  ancestor — and re-reads the document. Whatever a human sees is what the parser reads; no
  per-engine ad marker is needed. A card that is *visible* is still a hit.
- **A gate is not diagnosed.** `normal()` answers whether the DOM is the engine's own results page —
  parsed hits, or the engine's own "no results" wording. Consent walls, `/sorry/` traffic
  challenges, Baidu's security check and a half-rendered shell all answer "no", and the caller
  waits for a human instead of reasoning about which gate it is.

### Tabs

`tabs::POOL` is the process's only tab state.

```text
(session, TabKind::Search)  → search
(session, TabKind::Work)    → read, open, the actions, screenshot
```

- Tabs are created on demand and keyed by session and kind.
- A tab that vanished from the browser is recreated on the next call.
- Every tab — search and work — is serial per session through a slot lock held for the whole call,
  so two calls in one session queue on the tab rather than interleave on it. The pool lock only
  guards the maps; `/json/list`, `/json/new` and the CDP handshake happen outside it.
- A tab closes after 30 minutes without an active call. `active` counts the call in flight, so a
  busy tab is never reclaimed underneath itself.
- `close_session` closes both of a session's tabs, and `unbook` calls it: a session that ends —
  delete, or a sub-session finishing its task — takes its tabs with it.
- The `about:blank` the launch argv opens is reclaimed once a pool tab exists, so an owned browser
  shows the session's tabs and nothing else. An attached browser is never touched that way.

### Verification

A gate is not diagnosed: the provider only answers whether the DOM is its own results page
(`normal()` — hits, or the engine's own "no results" wording). Anything else is something a human
has to clear, and the domain waits instead of failing:

1. If `config.web.bringToFront` is true, the tab is raised once. The flag only decides whether the
   window comes forward; polling happens either way.
2. Every 30 seconds the current URL and DOM are handed to a predicate supplied by the caller; the
   default predicate is the provider's `normal()` answering "the results page is back".
3. Passing inside five minutes is silent: the page is fetched again and the operation continues as
   if nothing happened. The model never learns a verification occurred.
4. Five minutes without passing returns an error saying the user may be away or may not have
   noticed the window; the model is expected to ask the user, and a later retry starts over.

The poll never navigates, so the page the user is working on is not reset.

### The permit

At most two operations run at once across the process. The third waits on a condition variable.
The permit is taken once in `run`, so no operation can forget it. The count is also what the idle
sweep reads: a browser with an operation in flight is never closed.

### Output

`read` answers Markdown; with `raw` it writes the DOM to `<temp>/mypi/web/<stamp>.html` and answers
that path. `search` answers the provider, the page range, the count and a numbered list. The rest
answer a status line, except `screenshot`, which names its file and its size.

### What a read leaves out, and what it adds

The answer is the page's prose, not its document. Four things shape it:

- **Inline images go.** An `svg` element and an `img` whose `src` is a `data:` URI are dropped, and
  the alt text is kept where there is one. A marketing page carries its icons and sprites inline,
  and that payload routinely outnumbers the prose by a wide margin. An image naming a real address
  is left as it is.
- **The head's metadata goes.** No frontmatter is emitted at all. A page's `<meta>` tags are
  written for crawlers and for the site's own scripts: nonces, signatures, catalogue hashes, a
  visitor payload. None of it describes the article, and the article carries its own title and
  date in the text.
- **Landmarks go.** `<nav>`, `<footer>`, `<aside>` and `<form>` are removed, along with the
  elements whose class or role names them as chrome.
- **Controls stay, named.** `button`, `input`, `textarea` and `summary` are emitted as HTML where
  they sit, so the prose still shows a control was there, and the answer ends with an `operable`
  section naming each one's selector: the tool's `click` and `fill` take a selector, and Markdown
  has nowhere to put one.

A page whose body renders no text at all answers `target url has an empty body` rather than an
empty document, so the model can tell an unrendered page from a quiet one.

### Site rules

A site whose `<body>` mixes prose with furniture needs naming, and the naming lives in a file the
user owns: `<config dir>/web-rules.json`, beside `config.yaml`. A machine that has none is given
the built-in set (`rules.json` in this directory) on first use; from then on the file is what is
read, so a site deleted from it falls back to the generic conversion.

The file is one array of rules, tried in order. The first rule whose `match` — a regex over
`host + path`, never the query — matches the page is the rule that applies.

```json
[
  {
    "match": "^example\.com/doc/",
    "keep": [
      { "selector": "main", "drop": ["nav", "footer"] },
      { "selector": "#appendix", "drop": [] }
    ]
  }
]
```

- `keep` is the prose: each entry names one region, by a selector looked for from the body, and
the first element it names is what is read. Several entries are read in order and concatenated.
- `drop` applies inside its own entry only. A `nav` inside the kept region goes; a `nav` outside
it was never read.
- When no rule matches, or none of a rule's selectors resolve, the page is read whole: a broken
rule must not hide a page.

A site that writes no `<main>` gets no special handling. The regions above are how such a site is
named, and until it is, its whole body is what is read.

## 2. Design intent

- **A domain, not a tool.** The browser's behaviour is its own: when to start, how many tabs, when
  to reclaim them, when to die. The tool layer gives the model a convenient verb; it does not steer
  the browser.
- **Always headed.** A challenge needs a human in front of a window, and a headed browser carries
  fewer automation tells than any headless mode. This domain is for surfaces that need operating;
  plain text fetching has other tools.
- **One browser per process.** One persistent profile keeps logins; one window keeps the user able
  to supervise; one process group gives the exit path a single handle.
- **Owned versus attached.** A browser the user started is theirs; the domain only speaks CDP to it.
  A browser the domain started is a child of the server and must not survive it.
- **A resident fork thread.** `PR_SET_PDEATHSIG` binds to the forking thread, so a fork from a
  connection thread would kill the browser when that front end disconnects. One thread that never
  ends makes the signal mean "the server died".
- **Idle close.** The profile is on disk, so closing costs only a relaunch. Leaving the process up
  costs an orphan every time the server goes away badly.
- **Two tabs per session.** Search and interactive work have different lifetimes: a search tab may
  be replaced freely, a work tab must survive between `open`, `click`, `fill` and `read`. A session
  that ends closes both, so a long-lived browser never accumulates tabs from finished work.
- **A serial tab.** One call at a time per tab keeps a gate attributable to the engine that raised
  it, and keeps two calls from steering one page.
- **A silent verification.** The model asked for results; a human solving a challenge is
  infrastructure, not an outcome. Only a timeout is worth reporting.
- **30 seconds and five minutes are constants.** They are interaction pacing, not deployment
  parameters; a config key would invite tuning that makes the wait either useless or hostile.
- **Config read late and once.** Reading at launch keeps a long-lived session from re-parsing the
  file on every call, and keeps the domain from holding state it does not act on.

## 3. Unresolved

- macOS has no parent-death equivalent; only the explicit exit path covers it there.
- Windows needs a Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`); not written yet.
- Attached mode cannot switch a user's browser between headed and headless — irrelevant now, since
  there is no headless mode.
- The parsers are tuned against captured live pages; the engines edit their markup, so a
  parse that suddenly yields nothing is a selector problem, not a gate. Baidu's ads are not
  separated from organic results.
- A machine without a display cannot use this domain at all.

## 4. Config fields

Read by this domain itself, never through `Env`.

| key | use |
|---|---|
| `config.server.agent.tools.browser.bin` | the executable; empty = `$MYPI_BROWSER_BIN`, else `/opt/helium/helium` |
| `config.server.agent.tools.browser.profileDir` | the persistent profile; empty = `<data dir>/browser/profile` |
| `config.server.agent.tools.browser.port` | attach to a running browser instead of launching |
| `config.server.agent.tools.browser.idleSecs` | seconds of no activity before an owned browser is closed; default 600 |
| `config.server.agent.tools.browser.userAgent` | override the launched browser's User-Agent; unset = the browser's own |
| `config.server.agent.tools.browser.extraArgs` | extra argv |
| `config.web.bringToFront` | raise the window when a verification challenge appears; default true |
