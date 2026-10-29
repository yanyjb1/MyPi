# ipc

The wire between the two mypi processes: one byte stream, one JSON frame per line.

## Schema

A frame is a JSON object with three fields, always present:

```json
{"kind":"call","name":"init","payload":{"session":7,"limit":60}}
```

- `kind`: `register` | `call` | `post` | `subscribe` | `notice`.
- `name`: snake_case operation name. `kind` + `name` together are the route key.
- `payload`: always an object, possibly empty.

Requests and replies share `kind` and `name`; there is no id and no direction field. One
request on a connection, one reply in order. An error is a payload, not a kind:

```json
{"kind":"call","name":"init","payload":{"error":{"code":"bad_payload","message":"..."}}}
```

Four error codes: `unknown_name`, `bad_payload`, `busy`, `internal`.

The name table (ten `call` reads, thirteen `post` writes, three `subscribe` requests, eight
stream frames, one `notice`) lives in the api domain; this module does not know it.

## Design intent

- One line, one frame: a lost connection loses at most the frame in flight; a reader never
  needs buffering logic beyond `MAX_LINE_BYTES`.
- No id, no version: the main connection is strictly serial (mutex in the client), so a reply
  always answers the previous request. A version field would negotiate what a full reconnect
  decides better.
- Errors as payloads: the reply frame still echoes `kind` and `name`, so the requester needs
  no side channel to match an error to its request.
- ipc does not know names: the transport stays stable while the operation table evolves in
  the api. Every new operation is a change to two files, not three modules.

## Unresolved

- A stream connection dies with its process; there is no reconnect and no replay. The api's
  turn buffer covers only frames sent before `watch` returned.

## Config fields

None. ipc reads no config keys.
