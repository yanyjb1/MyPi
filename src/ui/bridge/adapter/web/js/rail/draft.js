// The draft: the directory, model and effort a session opens with, held until its row exists.

import { announce, dropPanel, slot, state } from "../base/state.js";
import { create } from "../base/write.js";
import { send } from "../chat/composer/talk.js";
import { enter } from "../chat/history/enter.js";

// Opening a draft writes nothing: the rail keeps its rows and the server keeps its count.
export function fresh() {
  if (state.follow) state.follow.close();
  dropPanel();
  state.follow = null;
  state.active = null;
  state.session = null;
  // The new draft keeps the directory and the model the last one stood on: only the effort starts over.
  const held = slot();
  state.draft = { cwd: held.cwd, model: held.model, effort: "auto" };
  announce();
}

export function isDraft() {
  return !state.session;
}

// A draft's first message makes the session it needs, then says what was typed.
export async function speak(text) {
  const held = slot();
  const session = await create(held.cwd, held.model, held.effort);
  if (session) await enter(session);
  await send(text);
}

export { slot };
