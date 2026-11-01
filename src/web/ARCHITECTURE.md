# web — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | declares the modules; exports `WebArgs`, `run`, `close_session` |
| `args.rs` | `WebArgs`: the shared input shape of every operation |
| `run.rs` | the two-operation permit, and the `op` dispatch |
| `url.rs` | URL normalisation, query encoding, percent and base64 decoding |

### browser

| file | holds |
|---|---|
| `browser/mod.rs` | declares the modules and the one re-export `search` needs |
| `browser/cdp.rs` | the CDP WebSocket transport: connect, send, receive, target discovery |
| `browser/process.rs` | one Chromium process or an attachment: launch argv, profile, target create and close, `port_alive` |
| `browser/page.rs` | one target: navigate, evaluate, read the DOM, poll, capture |
| `browser/session.rs` | the process-wide state: the handle, the tabs, the search locks, the challenge marks, `ensure_browser`, `ensure_headed`, `with_search`, `with_work`, `close_session` |
| `browser/reaper.rs` | one timer thread that closes idle tabs |
| `browser/open.rs` | `Page.navigate` |
| `browser/read.rs` | `DOM.getDocument` + `DOM.getOuterHTML` |
| `browser/actions.rs` | `Runtime.evaluate`, one shape per action |
| `browser/net.rs` | `performance.getEntriesByType('resource')` |
| `browser/screenshot.rs` | `Page.captureScreenshot` |

### search

| file | holds |
|---|---|
| `search/mod.rs` | the page loop, deduplication, the challenge handoff, the output format |
| `search/provider.rs` | `SearchProvider`, provider lookup, URL resolution, external-host filtering |
| `search/hit.rs` | `Hit`, the dedup key, first-seen deduplication |
| `search/providers/` | one module per engine, and the static provider table |

### html

| file | holds |
|---|---|
| `html/markdown.rs` | DOM → Markdown, and the JS-shell check |
| `html/tags.rs` | the tag scanner and entity decoder the providers use |

## What it talks to

- **`server::config`**: `Config::load()` and `BrowserOptions`, read inside `ensure_browser` and
  `ensure_headed`. This is the domain's only reach into the backend, and it is why `Env` carries no
  browser settings.
- **`platform`**: `env::data_dir` for the default profile root.
- **Crates**: `tungstenite` (WebSocket), `html-to-markdown-rs` (DOM to Markdown), `url`.
- **Who calls it**: `server::agent::tools::web` — `run` for every operation, and
  `tools::close_web_session` for a finished session.
- **Knows nothing about**: `Env`, tools, sessions' rows, the api doors, the front end.

## Inside

```text
run (permit)
  → read ──────→ browser::read::run ────→ session::with_search | with_work → Page
  → search ────→ search::run ──────────→ session::with_search, ensure_headed
  → open ──────→ browser::open::run ────→ session::with_work
  → actions ───→ browser::actions::run ─→ session::with_work
  → net ───────→ browser::net::run ─────→ session::with_work
  → screenshot → browser::screenshot::run → session::with_work
```

- `session` is the only owner of tab state. `page` does not know sessions. `process` does not know
  tabs. An operation does not touch CDP directly.
- `with_search` and `with_work` are the only doors into a page: they find or create the tab, take
  the lease, and hand the page to the caller's closure.
- `ensure_browser` reads the configuration at the point of launch only; a live handle returns
  before the read.
- `close_session` drops both tabs; the browser itself stays up for the process.

## Must stay in step

1. `WebArgs` and `tools/web/mod.rs`: the tool maps the model's positional arguments into this shape,
   so a field added here needs a parameter there.
2. `BrowserOptions` and `session::ensure_browser`: the domain reads the fields it acts on; a rename
   in `server::config` is a rename here.
3. `session::STATE` and `reaper`: the timer closes what the state records; a new tab kind needs both.
4. `search::provider` and `search/providers/`: the static table is the only registry, and `run`
   refuses a name it cannot find.
5. `page::RENDER_WAIT` and the providers' parsers: a page that has not rendered yields no hits, and
   the wait is what keeps the loop from reading a shell.

## Not wired yet

1. `close_session` has no caller; the api layer closes a session's tabs when it ends.
2. Provider selectors await live tuning.
3. A headed launch needs a display server; the failure is reported, not retried.
