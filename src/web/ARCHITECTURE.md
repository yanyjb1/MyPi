# web — architecture

What is here, what each file holds, who talks to whom. Current state only.

## Files

| file | holds |
|---|---|
| `mod.rs` | declares the modules; exports `WebArgs`, `run`, `close_session`, `shutdown`, `ENGINES` |
| `args.rs` | `WebArgs`: the shared input shape of every operation |
| `artifact.rs` | the files the model reads back: `<temp>/mypi/web/<stamp>`, one writer |
| `run.rs` | the two-operation permit, the in-flight count, and the `op` dispatch |
| `url.rs` | URL normalisation, query encoding, percent and base64 decoding |

### browser

| file | holds |
|---|---|
| `browser/mod.rs` | declares the browser's six parts |
| `browser/process/mod.rs` | one browser handle: owned or attached, port, `alive`, `stop`, and the profile's DevToolsActivePort lookup |
| `browser/process/launch.rs` | launch argv, the fork, the DevTools port wait, and the one resident thread that performs the fork |
| `browser/devtools/mod.rs` | `Client`: `/json/list`, `/json/new`, `/json/close`, port probing |
| `browser/devtools/cdp.rs` | the CDP WebSocket transport: connect, send, receive |
| `browser/devtools/target.rs` | `Target`: id and socket URL |
| `browser/page.rs` | one target: navigate, evaluate, DOM, wait, url, screenshot, bring-to-front |
| `browser/lifecycle/mod.rs` | the process-wide state: lazy start, ownership, `port`, `existing_port`, `shutdown` |
| `browser/lifecycle/idle.rs` | the activity clock and the idle decision |
| `browser/lifecycle/timer.rs` | the one 30-second thread: idle sweep, then tab reap |
| `browser/lifecycle/signal.rs` | INT/TERM/HUP registration |
| `browser/tabs.rs` | the tab pool, per-tab slots, leases, launch-tab reclaim, idle reap, `close_session` |
| `browser/verify.rs` | the human-verification poll: 30 seconds, five minutes, caller's predicate |

### ops

| file | holds |
|---|---|
| `ops/mod.rs` | declares the operations, and the shared action timeout |
| `ops/read.rs` | `read` |
| `ops/open.rs` | `open` |
| `ops/drive.rs` | `click`, `fill`, `select`, `scroll`, `eval` |
| `ops/screenshot.rs` | `screenshot` |

### search

| file | holds |
|---|---|
| `search/mod.rs` | the page loop, deduplication, the gate handoff, the output format, the engine list |
| `search/pace.rs` | the pauses between pages and between redirect lookups |
| `search/provider.rs` | `SearchProvider`, provider lookup, redirect settling, external-host filtering |
| `search/hit.rs` | `Hit`, the dedup key, first-seen deduplication |
| `search/providers/` | Google, Bing and Baidu, and the static provider table |

### html

| file | holds |
|---|---|
| `html/markdown.rs` | DOM → Markdown, and the JS-shell check |
| `html/tags.rs` | the tag scanner and entity decoder the providers use |

## What it talks to

- **`server::config`**: `Config::load()`, `BrowserOptions` and `bring_to_front()`, read inside
  `lifecycle::port` and `lifecycle::bring_to_front`. This is the domain's only reach into the
  backend, and it is why `Env` carries no browser settings.
- **`server::platform`**: `env::data_dir` for the default profile root, `proc::guard_orphan` before
  a fork, and `proc::terminate_tree` to close a group.
- **Crates**: `tungstenite` (WebSocket), `html-to-markdown-rs` (DOM to Markdown), `url`, `ureq`
  (DevTools HTTP), `signal-hook` (exit signals).
- **Who calls it**: `server::agent::tools::web` — `run` for every operation. `main` calls
  `shutdown` once the IPC server returns. `close_session` is called by the loop's `unbook`: a
  session that ends takes its tabs with it.
- **Knows nothing about**: `Env`, tools, sessions' rows, the api doors, the front end.

## Inside

```text
run (permit)
  → ops::read ────→ tabs::with_work → page
  → search ───────→ tabs::with_search → verify::await_human
  → ops::open ────→ tabs::with_work → page
  → ops::drive ───→ tabs::with_work → page
  → ops::screenshot → tabs::with_work → page

tabs::with_* → lifecycle::port → process::{launch, attach}
page → devtools::{Client, Cdp}
lifecycle::timer → idle::sweep, tabs::reap
lifecycle::signal → shutdown
```

- `tabs` is the only owner of tab state. `page` does not know sessions. `process` does not know
  tabs. An operation does not touch CDP directly.
- `lifecycle` is the only owner of the browser handle: `port` launches or attaches, `existing_port`
  never launches, `shutdown` closes and forgets. `tabs` asks it for a port and nothing else.
- `process::launch` is the only place that forks, and the only place that names a platform hook.
- `devtools` only speaks protocol: it never decides when a browser exists.
- `verify` knows a page and a predicate, not an engine.
- `idle` reads the permit count from `run`; the timer reads `idle` and `tabs`.

## Must stay in step

1. `WebArgs` and `tools/web/mod.rs`: the tool maps the model's positional arguments into this shape,
   so a field added here needs a parameter there.
2. `BrowserOptions` and `lifecycle::port`: an owned browser caches the idle timeout and the
   bring-to-front flag at launch; a rename in `server::config` is a rename here.
3. `tabs::POOL` and `lifecycle::shutdown`: the browser is gone, so every tab id is stale;
   `shutdown` clears the pool.
4. `search::provider` and `search/providers/`: the static table is the only registry, and `run`
   refuses a name it cannot find.
5. `page::RENDER_WAIT` and the providers' parsers: a page that has not rendered yields no hits, and
   the wait is what keeps the loop from reading a shell.
6. `platform::proc::guard_orphan` and `process::launch`: the fork must happen on the resident
   thread for the Linux parent-death signal to mean the server.
7. `tabs::close_session` and `loop::session::book::unbook`: every way a session ends must pass
   through `unbook`, or that session's two tabs outlive it until the 30-minute reap.

## Not wired yet

1. The three parsers are tuned against captured live pages, not guaranteed: the engines edit
   their markup, and a parse that suddenly yields nothing is a selector problem, not a gate.
2. Baidu's ads are not separated from organic results when they are visible: they are cards with
   real destinations and no stable marker in the capture. Hidden ones are deleted before parsing.
3. Windows has no Job Object yet; macOS has no parent-death equivalent.
