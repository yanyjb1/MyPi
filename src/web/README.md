# web

A self-contained domain for reading pages, searching, and driving a browser. The tool layer maps a
call's arguments into `WebArgs` and hands it over; everything below that line — Chromium, CDP, tabs,
providers, HTML conversion — belongs here and nowhere else.

It is not part of the tool layer. It decides for itself how the browser starts, which is why it
reads its own configuration instead of receiving it through `Env`.

## 1. Design / Schema

### One operation

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

`run(args, session)` is the whole entry. It takes one of two permits, dispatches on `op`, and
returns the text the model reads.

### Tabs

`session::STATE` is the process's only browser state: the handle, the tabs, the challenge marks,
and one search lock per session.

```text
(session, TabKind::Search)  → search, and a read that carries a url
(session, TabKind::Work)    → open, the actions, net, screenshot, a read without a url
```

- Tabs are created on demand and keyed by session and kind.
- A tab that vanished from the browser is recreated on the next call; its challenge mark is dropped
  with it.
- The search tab is serial per session through a lock held for the whole call, so two searches in
  one session queue rather than interleave.
- A tab closes after 30 minutes without an active call. The work tab is marked reserved for the
  duration of a call, so the call in flight is never reclaimed underneath itself.
- `close_session` closes both of a session's tabs.

### The permit

At most two operations run at once across the process. The third waits on a condition variable.
The permit is taken once in `run`, so no operation can forget it.

### Browser lifecycle

One Chromium per process, either spawned by this domain or attached to a port that already exists.

```text
ensure_browser
  → a live, attached or headless handle in STATE?  → reuse it
  → MYPI_BROWSER_PORT, config port, DevToolsActivePort → attach
  → otherwise                                        → launch
```

The profile is persistent: logins, cookies and user-installed extensions live there, and that is
the reason the browser exists at all.

Headless is the starting point. When a provider or a page answers with a challenge, the caller asks
`ensure_headed`: if the handle is attached it refuses, if it is already headed it does nothing, and
otherwise it replaces the handle with a headed launch. A launched browser stays headed for the
process; there is no switch back.

### Configuration

Read once, in `ensure_browser`, at the moment a browser has to start. A live handle short-circuits
before the read, so a long-lived session reads the file once and never again.

`BrowserOptions` comes from `server::config`; this domain never parses `config.yaml` itself.

| key | use |
|---|---|
| `config.server.agent.tools.browser.bin` | the executable to launch |
| `config.server.agent.tools.browser.profileDir` | the persistent profile |
| `config.server.agent.tools.browser.port` | attach instead of launching |
| `config.server.agent.tools.browser.headless` | launch without a window; default true |
| `config.server.agent.tools.browser.user_agent` | override the launched browser's User-Agent |
| `config.server.agent.tools.browser.extra_args` | extra argv |

`bin` and `profile_dir` have no usable default: a machine that names neither cannot start a
browser, and the failure is reported rather than papered over with a guessed path.

### Search

- One provider per call. `provider` is required; there is no aggregation across engines.
- Pages `1..=page` are fetched in order through the search tab; an empty page stops the loop.
- Hits are deduplicated by URL, first sighting wins.
- A challenge is a two-step handoff: detection returns a retry hint, the retry asks for a headed
  browser and the same provider is tried once more.
- Every provider reads through the browser. There is no plain-HTTP fallback.

### Output

`read` answers Markdown, or the DOM HTML with `raw`. `search` answers the provider, the page range,
the count and a numbered list. The rest answer a status line, except `net`, which lists resource
entries, and `screenshot`, which names a file and its size.

## 2. Design intent

- **A domain, not a tool.** The browser's behaviour is its own: when to start, headed or not, how
  many tabs, when to reclaim them. The tool layer gives the model a convenient verb; it does not
  steer the browser.
- **Flat `op`.** One verb per call, no nested action field, so the model never has to compose two
  operation names.
- **Two tabs per session.** Search and interactive work have different lifetimes: a search tab may
  be replaced freely, a work tab must survive between `open`, `click`, `fill` and `read`.
- **A serial search tab.** One provider, one tab, one call at a time keeps a challenge attributable
  to the engine that raised it.
- **A persistent profile.** An ephemeral profile would make the tool less useful and add cleanup
  work for no gain.
- **A permit of two.** Chromium serializes most CDP work anyway; two in flight keeps a batch moving
  without letting a fan-out of calls pile onto one browser.
- **Config read late and once.** Reading at launch keeps a long-lived session from re-parsing the
  file on every call, and keeps the domain from holding state it does not act on.

## 3. Unresolved

- `BrowserOptions` still carries six fields; only `bin` and `profileDir` are decisions a user has to
  make, and the rest are the domain's own defaults. The config rewrite settles this.
- Provider selectors are implemented but need live tuning against current result markup.
- A headed browser needs `DISPLAY` or `WAYLAND_DISPLAY`; an attached browser cannot switch to headed.
- CDP Network, HAR and response bodies are intentionally absent.

## 4. Config fields

All keys live under `config.server.agent.tools.browser`; see the table above. `bin` and
`profileDir` are required in practice — without them no browser starts.
