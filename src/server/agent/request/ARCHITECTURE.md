# request architecture

## Files

```text
request.rs                  public entry: start, hand_results, rebuild, ToolRun
content.rs                  request input/output, BlockSink, RetrySink
message.rs                  stored and wire message shapes
retry.rs                    retry policy, failure classes, retry notices

assembly/
  mod.rs                    build ProtocolRequest; insert system
  history.rs                stored Message -> WireMessage; tool-call grouping
  compact.rs                compaction plan, replay, summary request

protocols/
  mod.rs                    ProtocolId, Protocol, ProtocolRequest, WireRequest
  openai/
    completion/mod.rs       Chat Completions body and decode
    completion/compat/      completion-only compat switches
    deepseek.rs             DeepSeek request and decode
    responses.rs            Responses request and decode
    stream.rs               shared OpenAI SSE decode and tool-call stitch
    inband.rs               text-embedded tool-call scanner
    markup.rs               template marker filter
  anthropic/
    messages.rs             Messages request and decode
    stream.rs               Anthropic SSE event reader

transport/
  mod.rs                    WireRequest -> line reader
  client.rs                 provider connection, protocol, retry loop, worker slot
  worker.rs                 long-lived HTTP subprocess
```

## Relations

- `request::start` calls `rebuild`, `assembly::build`, and `Client::complete`.
- `Client::complete` asks the selected `Protocol` for a `WireRequest`, sends it through `transport`, and decodes the line reader through the same `Protocol`.
- Protocol decoders write `NewBlock` pieces to `BlockSink`. `request` forwards them to `Submit::Blocks`; storage writes and relays.
- `hand_results` turns `ToolRun` values into finished `tool_result` blocks and carries their `Effect` values in the same submission.
- `rebuild` reads storage and returns `Message` values. `assembly::history::to_wire` is the only stored-to-wire translation.
- `assembly::compact` builds a `ProtocolRequest` for the summary callback; it does not write compaction rows.
- `transport::send` uses the client worker when present, otherwise a direct HTTP request. It does not retry.
- `Client::complete` owns retry, cancellation, and worker replacement.
- `protocols` does not know storage tables or writer details; it only produces `NewBlock` pieces and `AssistantMessage`.
- `request` does not read config files or open database connections. The caller supplies the connection, client, retry policy, and cancellation flag.

## Callers

- The API/loop layer calls `start` and `hand_results` and supplies `RequestContent`.
- The context tool calls `rebuild`.
- Config supplies the retry policy and protocol id.
- Storage supplies history and consumes `Submit::Blocks`.
