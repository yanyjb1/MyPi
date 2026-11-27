// Entering a conversation: its stream first, then its rows, so no frame lands before its shape.

import { follow } from "../../base/wire.js";
import { announce, dropPanel, ensure, state } from "../../base/state.js";
import { bill, open } from "../../base/read.js";
import { windowed } from "../../base/model.js";
import { absorb, forget } from "./live.js";

export async function enter(id) {
  if (!id) return;
  if (state.follow) state.follow.close();
  if (state.active !== id) dropPanel();
  const held = ensure(id);
  state.active = id;
  state.refusal = null;
  state.session = held;
  state.pinned = true;
  held.anchor = null;
  // The stream replays the round in flight, so the tail starts empty and every frame it says fills it again.
  held.live = [];
  held.liveNode = null;
  forget();
  held.busy = false;
  held.liveTurn = null;
  announce();
  state.follow = follow(id, (frame) => absorb(held, frame), () => {});
  state.streaming = true;
  await open(id);
  state.streaming = false;
  await Promise.all([bill(id), windowed()]);
}
