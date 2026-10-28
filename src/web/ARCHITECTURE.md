# web architecture

## Entry points

- `mod.rs` exposes `WebArgs`, `run`, and `close_session`.
- `args.rs` defines the shared input shape.
- `run.rs` enforces the global two-operation limit and dispatches `op`.
- `url.rs` owns URL normalization, query encoding, percent decoding, and base64 decoding.

`src/server/agent/tools/web` calls `run(&WebArgs, &BrowserConfig, session)`. The API layer calls `close_session(session)` when a session ends. No other module needs to know about CDP or providers.

## Browser

- `browser/cdp.rs` owns the CDP WebSocket transport, target discovery, navigation, evaluation, and DOM reads.
- `browser/process.rs` owns one Chromium process or attachment, the profile path, target creation, and target closing.
- `browser/page.rs` wraps one target. It navigates, evaluates, reads the DOM, polls, and captures screenshots.
- `browser/session.rs` owns process-wide state: the browser, per-session search/work tabs, active counters, reserve flags, challenge marks, and per-session search locks.
- `browser/reaper.rs` starts one timer thread that closes idle tabs.
- `browser/open.rs`, `read.rs`, `actions.rs`, `net.rs`, and `screenshot.rs` are the operations over `Page`.

`session` is the only owner of tab state. `page` does not know sessions. `process` does not know tabs. Operations do not touch CDP directly.

## Search

- `search/mod.rs` runs the page loop, deduplication, challenge handoff, and output formatting.
- `search/provider.rs` defines `SearchProvider`, provider lookup, URL resolution, and external-host filtering.
- `search/hit.rs` defines `Hit`, deduplication keys, and first-seen deduplication.
- `search/providers/` holds one module per provider and the static provider table.

`search` depends on `browser` for the search tab and on `html`/`url` for parsing helpers. Providers depend only on `provider`, `hit`, `html`, and `url`. Providers do not call each other and do not fetch HTTP.

## HTML

- `html/markdown.rs` converts DOM HTML to Markdown and detects JS shells.
- `html/tags.rs` is the tag-level scanner and entity decoder used by providers.

`html` knows nothing about browsers, sessions, or config.

## Relationships

- `run` calls `browser` or `search`.
- `browser` operations call `session`, which calls `process` and `page`.
- `search` calls `browser::with_search` and `browser::ensure_headed`.
- `session` calls `reaper::start` after a browser starts.
- `reaper` calls `session::reap`.
- `process` calls `url::encode_query` for target creation.
- `page` calls `process` for reconnect and `url::base64_decode` for screenshots.

## Not wired here

- Provider parsers are implemented but await live selector tuning.

The domain does not read the config file, the database, or `Env`; it receives `BrowserConfig` and a session id.
