# EXTRA

This file records the remaining implementation deviations from the approved target form.

## 1. `subscribe::connect`

The target form lists `watch`, `submit`, and `compact`. `watch` registers a session and returns an
ack; it does not create a channel. A stream needs a receiver before the first event can be
delivered, so `connect` creates the viewer's one stream and hands the receiver to the caller.

This is plumbing, not a fourth subscription. The target's three entries still describe what a front
end can ask for. If IPC later owns the channel, `connect` should be folded away.

## 2. Config registry

`Call::Server::Config` answers from a wire `ConfigValue` and resolves global keys by walking the
serialized `AppConfig`; `model.*` and `profile.*` resolve against the attached session.

The registry does not exist yet. When it lands, the serialized traversal should be replaced by a
registry lookup with key kinds and defaults. The wire enum itself belongs to `ipc::contract` and is
not a deviation.
