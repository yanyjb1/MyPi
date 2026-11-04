# agent architecture

## Files

```text
loop/       sessions, turns, mailboxes, pen, live line
request/    outgoing request, stream decode, block cut, request archive
tools/      tool table, Env, one folder per tool
mod.rs      the three module declarations
```

## Relations

- `loop` calls `request::Ask` for one lap and `tools::Batch` for one tool batch.
- `request` writes blocks through the sink the loop hands in; it does not write turn rows.
- `tools` reads and writes only through `Env`; the loop writes any resulting rows.
- `storage` is reached by `loop` through `Writer` and `Reader`; request and tools do not see it.
- `config` gives the loop resolved values; request and tools receive values in their inputs.
