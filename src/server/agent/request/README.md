# request

## 1. Design / Schema

`request` turns one stored session into one model request, decodes the reply, and forwards block pieces to the storage writer.

Public surface:

- `start(w, r, client, content, retry, cancel) -> Answer`
- `hand_results(w, session, reply, ran)`
- `rebuild(r, session) -> Vec<Message>`
- `ToolRun`
- `content::{RequestContent, Answer, BlockSink, RetrySink}`
- `message::{Message, WireMessage, AssistantMessage, ToolCall, Usage, StopReason}`
- `protocols::{ProtocolId, Protocol, ProtocolRequest, WireRequest}`

`ProtocolId` accepts `openai-completion`, `openai-deepseek`, `openai-responses`, and `anthropic-messages`.

`RequestContent` carries `session`, `system`, `tools`, `max_output_tokens`, and `profile`.

`ProtocolRequest` carries `model`, `max_output_tokens`, `stream`, `messages`, `tools`, and `session_id`.

`WireRequest` carries `url`, `headers`, and `body`.

`BlockSink::block(block, done)` receives `NewBlock` pieces. `done = false` is an increment; `done = true` closes the block and must carry the complete block content. The protocol decoder decides `NewBlock.kind` and `done`; request only stamps `ts` and forwards to `Submit::Blocks`.

`RetrySink::retry` receives `RetryNotice` with `level`, `reason`, `attempt`, and `max`.

`ToolRun` carries `call_id`, `ok`, `result`, `memo`, `spill`, `duration_ms`, `effects`, and `stop`. `effects` are storage write intents; `hand_results` sends them with the finished tool-result blocks.

`RequestMeta` stores `model`, `provider`, `protocol`, `base_url`, `profile`, `system`, `tools_json`, `max_tokens`, and `compat_json`. `protocol` is a hyphen id. `profile` is a name, not a fingerprint.

## 2. Design Intent

- Upstream owns block shape. The protocol decoder knows the wire format, so it decides block type and boundary. Request and storage do not recognize the wire again.
- Text and tool calls are separate blocks. Text is `assistant`; each call is `tool_request`. `history::to_wire` merges an adjacent assistant text block into the first call when replaying the wire message.
- Request does not hold conversation state. Every request rebuilds history from storage.
- System is inserted only by assembly; stored messages never contain it.
- Retry belongs to one request. It covers no-response and HTTP 429 only; it stops after any emitted block or cancellation. `maxRetries` counts retries after the first request; the interval is fixed and `Retry-After` is ignored. A 429 notice is `Error` with the upstream body; a timeout notice is `Warn` with `request timeout: <timeout>`. Notices are not stored.
- Effects ride the same `Submit::Blocks` as their tool-result blocks, so a tool result and its memo/alias/subsession writes land together.
- Empty and interrupted replies are block events produced at decode end, not post-hoc cuts.
- Compaction is a prepared request: `plan` chooses the cut, `replay_context` builds the live context, and `compact` sends the summary request.

## 3. Unresolved

No open items inside this domain.

## 4. Config Fields

- `config.server.agent.request.maxRetries` — retries after the first request; default 10.
- `config.server.agent.request.retryInterval` — fixed wait between retries; default 30s.
- `config.server.agent.request.timeout` — first-byte wait before killing the worker; default 120s.
