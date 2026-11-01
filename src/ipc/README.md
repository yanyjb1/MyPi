# ipc

The wire between the two mypi processes: one byte stream, one JSON frame per line.

## Schema

A frame is a JSON object with three fields, always present:

```json
{"kind":"call","name":"init","payload":{"session":7}}
```

- `kind`: `register` | `call` | `post` | `subscribe` | `stream`.
- `name`: snake_case operation name. `kind` + `name` together are the route key.
- `payload`: always an object, possibly empty.

A reply repeats the request's `kind` and `name`. An error is a payload, not a kind:

```json
{"kind":"call","name":"init","payload":{"error":{"code":"bad_payload","message":"..."}}}
```

Push frames use `kind = stream`. A reply repeats the request's `kind`, so `kind` alone tells a
reply from a push without an id, and a push may reuse a door's name.

The name table and the error codes belong to api; this module knows neither.

## Design intent

- One line, one frame: a lost connection loses at most the frame in flight; a reader never
  needs buffering logic beyond `MAX_LINE_BYTES`.
- One connection per front end, two directions: one thread reads, one thread writes. Replies
  and pushes share the writing side, so the socket is written in one place only.
- No id, no version: requests on a connection are serial, so the reply that matches the
  request's `kind` and `name` is its answer. Stream frames carry another kind and are never
  mistaken for one.
- ipc does not know names or payloads: the transport stays stable while the operation table
  evolves in api.

## Unresolved

- A dropped process has no reconnect and no replay; the api's stream buffer covers only what
  arrived while the connection lived.

## Config fields

None. ipc reads no config keys.
