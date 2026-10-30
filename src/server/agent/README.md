# agent

The agent domain has three parts:

```text
storage ──► loop ──► { request, tools }
```

- `loop/` is implemented. It owns sessions, turns, mailboxes, row writes and the live line.
  See `loop/README.md` and `loop/ARCHITECTURE.md`.
- `request/` turns one outgoing request into a stream, cuts the reply into blocks and writes them.
  It does not open turns, run tools or touch session state.
- `tools/` is a function table called through `Env`. It does not open the database, read config,
  touch environment variables or hold a `Writer`.

The dependency is one-way: `loop → request` and `loop → tools`. Request and tools do not know each
other.

The unfinished cross-domain contract is in `loop-域外要求.md`. The remaining high-level decisions
are in `AGENT-DECISION.md`. Each subdomain keeps its own `README.md` and `ARCHITECTURE.md`.
