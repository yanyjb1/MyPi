# api

## Design

The api sits between the ipc server and the rest of the backend. It has three jobs, and nothing else belongs here:

- **Doors.** `call` answers from the database, `post` changes session data, `subscribe` changes who follows what.
- **Reader.** Every `call` opens its own read connection and closes it.
- **Stream.** One buffer per session, one outbound queue per connection.

### Wire

A frame is `{"kind", "name", "payload"}`; `payload` is JSON and api defines no types of its own.
Kinds: `register`, `call`, `post`, `subscribe`. A reply repeats the request's kind and name.
A refusal is a reply whose payload is `{"error": {"code", "message"}}`; codes are
`bad_payload`, `busy`, `permission_denied`, `internal`.

### Doors

`call`: `init`, `blocks`, `meta`, `usage`, `todo`, `memo`, `children`, `config`, `pools`, `sessions`.
`post`: `init`, `name`, `cwd`, `switch`, `profile`, `todo_on`, `todo_off`, `delete`, `interrupt`,
`rewind`, `default_model`, `reload`.
`subscribe`: `watch`, `submit`, `compact`.

### Stream

Hello opens the connection's stream and answers `register.opened`. Everything after that is pushed
under kind `subscribe`:

- `post` — a session-data change, carrying its issuer. The issuer never reads its own write back.
- `stream` — block content produced by the backend.
- `notice` — the server's own line.
- `structure` — the closing event of a turn: the round id and the block id ranges it owns.

Rules:

- A buffer exists only while the session has a subscriber. The first event with a subscriber creates
  it; the last subscriber leaving destroys it. No subscriber, no record.
- The buffer holds everything since its last clear, whatever the event kind.
- It clears once: when the turn ends and its `structure` event has been handed to the current
  subscribers.
- Replies and stream events share one outbound queue per connection; one thread writes the socket.

### Roles

`submit` and `compact` take the session's writer slot; a new writer demotes the old one. A writer
keeps the slot until it is taken, switches session, or disconnects. `post` is refused with
`permission_denied` unless the caller holds that session's slot. `watch` only declares a session;
a connection follows one session at a time, and switching is the only way to stop following.

## Intent

- The buffer belongs to the session, not to a connection: a front end may attach mid-turn, and a
  reconnect must not change what the turn already produced.
- `structure` is the clear marker because a turn is not finished for a reader until it knows which
  blocks the turn owned.
- Push frames reuse kind `subscribe` so the registry stays four kinds and no request id is needed.
- api holds no wire types: the I/O document is the contract, so a new field never reaches across
  domains.

## Unresolved

- `config`, `pools`, `default_model`, `reload` need the config, which lives outside this region.
  They refuse with `internal` until the config is wired.
- `submit` and `compact` expect the loop's `deliver` to return the new round id. The loop's current
  signature returns nothing.
- `notice` has no caller yet; the loop's outlets stay silent.
- The ipc server still owns the socket write path; it must hand each connection a queue and drain it
  on its own thread, and call `Api::connect` / `Api::disconnect`.

## Config keys

None.
