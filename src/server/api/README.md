# API

`src/server/api` implements the wire contract in `ipc::contract`; it owns the semantics, not the
shapes. Storage, config, agent and the front ends live behind it.

## 1. Design / Schema

### Doors

- `call` — a read-only question, answered once.
- `post` — a state change, answered with a receipt.
- `subscribe` — a viewer's stream, and the two writes that open a round on it.
- `notice` — the server's outbound path; not a door.

### Call

```text
Call::Session { session, ask: SessionAsk }
    SessionAsk::Init { limit }
    SessionAsk::Blocks { ranges }
    SessionAsk::Meta
    SessionAsk::Usage { lang }

Call::Tool(ToolAsk)
    ToolAsk::Todo { session }
    ToolAsk::Memo { session, op, scope, name }
    ToolAsk::Children { session }

Call::Server(ServerAsk)
    ServerAsk::Config { key }
    ServerAsk::Pools
    ServerAsk::Sessions { under }
```

### Post

```text
Post::Session { session, write: SessionWrite }
    SessionWrite::Name, Cwd, Switch, Profile, TodoOn, TodoOff, Delete, Interrupt, Rewind, Unwatch

Post::Tool(ToolWrite)
    ToolWrite::Memo(MemoWrite)

Post::Server(ServerWrite)
    ServerWrite::DefaultModel, Reload

Post::Init { cwd, model, name, profile }
```

`PostDone` is `Done`, `Created(id)`, or `Busy`.

### Subscribe

```text
Subscribe::Watch { session }
Subscribe::Submit { session, prefix, text, suffix }
Subscribe::Compact { session, retention, focus }
```

`Opened` is `Started { turn }` or `Busy`. `Retention` is `Default`, `Tokens(n)`, or `Blocks(n)`.

### Replies and events

```text
Reply::Session(SessionReply) | Tool(ToolReply) | Server(ServerReply)

SessionReply::Init(InitReply) | Blocks(Vec<Block>) | Meta(SessionState) | Usage(UsageReply)
ToolReply::Todo(Vec<TodoPhase>) | Memo(MemoData) | Children(Vec<ChildRow>)
ServerReply::Config(ConfigValue) | Pools(CandidatePools) | Sessions(Vec<SessionRow>)

Event::Content(ContentEvent) | Session(SessionEvent) | Notice(Notice)

ContentEvent::Started | Live | Blocks | ToolOutput | Ended
SessionEvent::Meta | Rewound | Deleted
```

`StartKind` is `Submit`, `Compact`, or `Wake`.

### The stream

- `subscribe::connect` creates one stream per viewer.
- `Watch` points that stream at a session and receives every turn.
- `Submit` and `Compact` point it at one turn (session + turn).
- `ContentEvent` carries the turn; `SessionEvent` and `Notice` are session-wide.
- The API keeps only the current turn's increments. A viewer that joins mid-turn receives the
  current turn's backlog first, then increments.
- `Ended` closes the turn: its buffer and its turn subscriptions are dropped.
- There is no history in the API. History is read with `Call::Session::Init` and
  `Call::Session::Blocks`.

### Viewer

`Viewer` is a UUID string made by the front end. The server keeps viewer → session for the
attached session; a `Call::Server::Config` for `model.*` or `profile.*` resolves against it.
The viewer is carried by the connection, never by a `Call`, `Post`, or `Subscribe` value.

### Query rules

- `Init`: the whole structure from the read start, newest round first, plus the newest `limit`
  blocks in ascending id order. When the session's `todo_on` flag is true and the replay is
  non-empty, it also carries `todo`; otherwise the field is absent. It does not return
  `todo_on` itself.
- `Blocks`: closed ranges, in request order, ascending inside a range, overlaps kept. A range
  outside the session is an error.
- `Meta`: the session row plus the live working directory.
- `Usage`: one row per model, priced from the stored tokens and the config's rates.
- `Todo`: the session's list, replayed from its own `todo` calls.
- `Memo`: names, current content, or the full history; `current` needs a session, `global` does
  not. The single-note operation is `show`; `history` returns every version.
- `Children`: the child sessions a `task` call spawned, with anchor, row, intent, and state.
- `Config`: a registry-shaped value; `model.*` and `profile.*` use the attached session, global
  keys ignore it; an unknown key is an error.
- `Pools`: model ids and profile names for completion.
- `Sessions`: resume rows, newest first, with child sessions filtered out.

### Write rules

- `Post` is the only write door.
- `Delete` and `Rewind` are refused while the session is mid-round; `Name`, `Cwd`, `Switch` and
  `Profile` are not.
- `Interrupt` only sets the running round's stop flag.
- `Unwatch` drops the viewer's stream.
- `Delete` refuses a child session; deleting a parent deletes its children and stops their jobs.
- `Rewind` only prunes; the fork mode is not built.
- `Reload` is refused while any session runs, replaces the config, drops idle `Live`s, and rebuilds the MCP bridge.
- `Init` is the only write that answers with a new id.
- `TodoOn` and `TodoOff` are transition writes: one `true` at the start, one `false` at the
  end, not one per `todo` toolresult. Both carry the `call_id` of the relevant `todo` toolresult.
  `TodoOn` writes `sessions.todo_on = true` and the literal `true` into that block's `memo`;
  `TodoOff` writes `false` into the flag and the memo. The front end is the only maintainer of
  the list; the backend does not infer or repair the flag.
- A memo `add` refuses an existing name; `mod` appends a version; `del` removes every version.
  Identity is `scope + name`; `anchor` is provenance. A `current` write carries the latest landed
  block as its anchor and that block must belong to the owning session; `global` carries no anchor.

## 2. Design intent

- **Three doors, not one.** A read, a write, and a stream have different failure and lifetime
  rules; one enum would force every caller to carry the other two.
- **Groups and queries.** A door names a business group; the group names one operation. Adding a
  query touches its group, not the door or the other groups.
- **Grouped replies and events.** The top level carries only group tags, so the wire vocabulary
  stays stable while a group grows.
- **One stream per viewer.** A session's content is one stream; subscriptions are filters over it.
  Two front ends can watch the same session without duplicating the producer.
- **Current-turn buffer, no history.** The stream is for live increments; history is a read. This
  keeps the API from becoming a second transcript store.
- **Backlog on join.** A viewer that starts watching mid-turn still sees the turn from its first
  event; `Ended` is the only cleanup point.
- **Notice is not a door.** Doors are front-end requests; a notice is the server speaking. It
  shares the session stream but has no group or query.
- **Viewer on the connection.** The front end's identity belongs to its connection, not to every
  request; the server keeps the mapping.
- **Config follows the attached session.** `model.*` and `profile.*` are session-relative;
  global keys are not. The caller does not pass a session to a config read.
- **Child sessions are sessions.** They reuse the loop, storage, and relay; the API only refuses
  a direct submit and a direct delete, and lists them under their anchor.
- **Memo is a tool.** Reads are `Call::Tool`; writes are `Post::Tool`; identity is `scope + name`
  and the anchor is only provenance. A `current` write uses the latest landed block; `global` has
  no anchor.
- **Todo is front-end state.** The front end owns the list and the on/off transition; the
  backend only stores the flag and the transcript marker. `Init` gives the initial list once,
  and only when it is non-empty.

## 3. Unresolved

- The config registry is not built. The wire `ConfigValue` lives in `ipc::contract`; global keys
  are resolved by walking the serialized app config, and the registry should replace that lookup.
- The compaction summary is landed by storage as its own round; the API ends the fixed
  `/compact` turn itself. Storage and the API must agree on which turn a compaction's blocks
  belong to.
- Memo writes use a transaction on a read/write connection; the writer does not own them yet.

The three implementation deviations from the target form are recorded in `EXTRA.md`; none of them
is a design goal.

## 4. Config fields

| Key | Use |
|---|---|
| `config.server.api.defaultModel` | Default model written by `Post::Server::DefaultModel`; read by `Config`. |
| `config.server.storage.batchTools` | Writer batch size at driver open. |
| `config.server.agent.profile` | Active profile when a session has none. |
| `config.server.agent.loop.maxToolCalls` | Fence put into each `Live`'s `ToolBudget`. |
| `config.server.agent.tools.*` | Output limits, browser, bash timeout, and job limits for each `Live`. |
| `config.server.agent.subagent.maxToolCalls` | Default child budget; `task` may override it. |
| `config.server.request.compact.*` | Compaction plan and instruction. |
| `config.server.pruneOnRewind` | Must be true; the config loader refuses the fork mode. |
| `config.ui.*`, `config.tui.*` | Reachable through `Call::Server::Config`; owned by the front ends. |
