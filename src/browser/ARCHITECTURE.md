# browser — architecture

What is here, what each file holds, and who talks to whom; current state only.

## Files

- `mod.rs` — declares the modules, and exports `WebArgs`, `Setup`, `run`, `close_session`, `shutdown` and `ENGINES`.
- `args.rs` — `WebArgs`, the shared input shape of every operation.
- `artifact.rs` — the files the model reads back: one writer, named for the site and numbered.
- `halt.rs` — the stop as this domain says it: `Stopped`, and whether an error is it.
- `seen.rs` — what each session last read: one record per session, its address and its prose.
- `run.rs` — the two-operation permit, the in-flight count, and the `op` dispatch.
- `url.rs` — URL normalisation, query encoding, percent and base64 decoding.

### The browser itself

- `process/mod.rs` — one browser handle: owned or attached, port, `alive`, `stop`, and the profile's DevToolsActivePort lookup.
- `process/launch.rs` — launch argv, the fork, the DevTools port wait, and the one resident thread that performs the fork.
- `devtools/mod.rs` — `Client`: `/json/list`, `/json/new`, `/json/close`, port probing.
- `devtools/cdp.rs` — the CDP WebSocket transport: connect, send, receive, and the `Knocker` a page's waits answer.
- `devtools/target.rs` — `Target`: id and socket URL.
- `page.rs` — one target: navigate, evaluate, DOM, wait, url, screenshot, bring-to-front.
- `lifecycle/mod.rs` — the process-wide state: lazy start, ownership, `port`, `existing_port`, `shutdown`.
- `lifecycle/idle.rs` — the activity clock and the idle decision.
- `lifecycle/timer.rs` — the one 30-second thread: idle sweep, then tab reap.
- `lifecycle/signal.rs` — INT/TERM/HUP registration.
- `tabs.rs` — the tab pool, per-tab slots, leases, launch-tab reclaim, idle reap, `close_session`.
- `verify.rs` — the human-verification wait: 30 seconds between looks, five minutes at most.

### ops

- `ops/mod.rs` — declares the operations, and the shared action timeout.
- `ops/read.rs` — `read`.
- `ops/open.rs` — `open`.
- `ops/drive.rs` — `click`, `fill`, `select`, `scroll`, `eval`.
- `ops/screenshot.rs` — `screenshot`.

### search

- `search/mod.rs` — the page loop, deduplication, the gate handoff, the output format, the engine list.
- `search/pace.rs` — the pauses between pages and between redirect lookups.
- `search/provider.rs` — `SearchProvider`, provider lookup, redirect settling, external-host filtering.
- `search/hit.rs` — `Hit`, the dedup key, first-seen deduplication.
- `search/providers/` — Google, Bing and Baidu, and the static provider table.

### html

- `html/markdown.rs` — DOM to Markdown, the SVG and inline-image drops, and the JS-shell check.
- `html/control.rs` — `expression` fills the scan's one placeholder, `scan` reads its answer, and `section` writes the control list.
- `html/control.js` — the scan itself, compiled in: one site rule applied once, answering the kept HTML and every control that survived in it.
- `html/tags.rs` — the tag scanner and entity decoder the providers use, and the two cuts the conversion begins with, SVG and controls reduced to their text.

## What it talks to

- **`server::config`**: the value types `BrowserOptions`, `Rules` and `Part`, and nothing else; every one of them arrives as a value from the caller, so this domain opens no configuration file.
- **`platform`**: `env::data_dir` for the default profile root, `proc::Process` for the browser's tree (spawn, stop, wait), and `stop::Stop` for the turn's stop signal, which ends the wait a call sits on rather than being polled.
- **Crates**: `tungstenite` for the WebSocket, `html-to-markdown-rs` for DOM to Markdown, `url`, `ureq` for the DevTools HTTP, and `signal-hook` for the exit signals.
- **Who calls it**: `server::agent::tools::web` and `server::agent::tools::search`, which call `run` for every operation, and the loop's `unbook`, which calls `close_session` so a session that ends takes its tabs with it; `main` calls `shutdown` once the IPC server returns.
- **Knows nothing about**: the api doors, storage, the front end.

## Inside

```text
run (permit, Setup)
  → ops::read ────→ tabs::with_work → page
                   rules → html::control::expression → evaluate → html::control::scan
  → search ───────→ tabs::with_search → verify::await_human
  → ops::open ────→ tabs::with_work → page
  → ops::drive ───→ tabs::with_work → page
  → ops::screenshot → tabs::with_work → page

tabs::with_* → lifecycle::port(Setup) → process::{launch, attach}
page → devtools::{Client, Cdp}
lifecycle::timer → idle::sweep, tabs::reap
lifecycle::signal → shutdown
```

- `tabs` is the only owner of tab state, `page` does not know sessions, `process` does not know tabs, and an operation does not touch CDP directly.
- `lifecycle` is the only owner of the browser handle: `port` launches or attaches, `existing_port` never launches, `shutdown` closes and forgets, and `tabs` asks it for a port and nothing else.
- `Setup` is the one thing this domain takes from the outside: the launch values and the bring-to-front switch, decided by whoever loaded the configuration.
- `process::launch` is the only place that forks, and the only place that names a platform hook.
- `devtools` only speaks protocol, and it never decides when a browser exists.
- `verify` knows a page and a predicate, not an engine.
- `halt` is the one place the stop is named, where an error downcasts to `Stopped` so that no layer reads text.
- `idle` reads the permit count from `run`, and the timer reads `idle` and `tabs`.
- `html::control` scans the live page through `evaluate`, so its expression and `drive`'s selectors meet at the same engine: what the scan names, `click` and `fill` can reach.
- `html::control` applies a site rule once, in the page, and answers the kept HTML together with the controls still inside it, and the prose and the `operable` section come from that one answer.
- `control.js` names a control once per chain: a widget beats the shells around it, a bare shell answers by the innermost one, and a shell whose subtree holds a widget leaves it to that widget.

## Stopping

The turn's stop reaches this domain as `platform::stop::Stop`, passed from the tool entry into `run` and down to whatever waits, and no wait in here polls it: each one registers what ending it means, and wakes on that alone.

- A page's CDP connection carries a `Knocker`, and a stop aborts it, so the call waiting on the wire answers `halt::Stopped` at once instead of running to its timeout; the `Knocker` belongs to the page rather than to one socket, so a stop outlives the reconnect that replaces it.
- A pause, whether between result pages, between redirect lookups, waiting for a human to clear a verification or queuing for a permit, is `Stop::pause`.
- A stopped call hands over what it already produced: the DOM on the tab becomes prose, the pages already fetched become the search answer, and the settled hits stand.

The front door turns the stop into the result's reason, and this domain never writes that marker itself.

## Must stay in step

1. `WebArgs` and `tools/web/mod.rs`: the tool maps the model's positional arguments into this shape, so a field added here needs a parameter there.
2. `Setup` and `lifecycle::port`: an owned browser caches the idle timeout and the bring-to-front flag at launch, and the tool layer reads both from the configuration once.
3. `tabs::POOL` and `lifecycle::shutdown`: the browser is gone, so every tab id is stale, and `shutdown` clears the pool.
4. `search::provider` and `search/providers/`: the static table is the only registry, and `run` refuses a name it cannot find.
5. `page::RENDER_WAIT` and the providers' parsers: a page that has not rendered yields no hits, and the wait is what keeps the loop from reading a shell.
6. `platform::proc::Process` and `process::launch`: the fork must happen on the resident thread for the Linux parent-death signal to mean the server.
7. `tabs::close_session` and `loop::session::book::unbook`: every way a session ends must pass through `unbook`, or that session's two tabs outlive it until the 30-minute reap.

## Not wired yet

1. The three parsers are tuned against captured live pages rather than guaranteed, because the engines edit their markup and a parse that suddenly yields nothing is a selector problem, not a gate.
2. Baidu's ads are not separated from organic results when they are visible, because they are cards with real destinations and no stable marker in the capture, and hidden ones are deleted before parsing.
3. Windows has no job object yet, and macOS has no parent-death equivalent.