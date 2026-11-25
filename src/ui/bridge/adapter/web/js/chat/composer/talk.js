// One message out: a new round through `subscribe`, a queued line through `post`.

import { post, subscribe } from "../../base/wire.js";
import { announce, failed, state } from "../../base/state.js";

export function send(text, prefix) {
  const body = String(text || "");
  if (!body.trim()) return;
  const held = state.session;
  if (!held) return;
  const payload = { session: held.id, text: body };
  if (prefix) payload.prefix = prefix;
  const running = held.busy;
  held.busy = true;
  let mine = null;
  if (running) {
    held.queued.push(body);
  } else {
    // The api skips the issuer of a round's opening words, so this page draws its own line.
    mine = { type: "user", text: body };
    held.live.push(mine);
  }
  announce();
  const quiet = running
    ? post("mailbox", payload)
    : subscribe("submit", payload);
  quiet.catch((e) => {
    if (running) {
      held.queued = held.queued.filter((one) => one !== body);
      held.busy = held.queued.length > 0;
    } else if (mine) {
      held.live = held.live.filter((one) => one !== mine);
      held.busy = false;
    }
    announce();
    failed(e);
  });
}
