// Entering a conversation and starting one: the stream first, then the rows, so no frame lands before its shape.
// A draft is nothing the server knows about until its first word, which is why it is made here too.

import { bill, create, follow, open, windowed } from "../../core/api.js";
import { at, clear } from "../../core/template.js";
import { announce, dropPanel, ensure, state, slot } from "../../core/store.js";
import { send } from "../composer/talk.js";
import { absorb, forget } from "./stream.js";

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
export async function speak(line) {
  const held = slot();
  const session = await create(held.cwd, held.model, held.effort);
  if (session) await enter(session);
  await send(line);
}

// The head of the middle column: which conversation is on screen, and whether it is still answering.
export function head() {
  const held = state.session;
  at("main").dataset.empty = held ? "0" : "1";
  at("topbar").querySelector("h1").textContent = held
    ? (held.meta && held.meta.name) || "session " + held.id
    : "mypi";
  const acts = clear(at("session-acts"));
  if (!held || !held.busy) return;
  const running = document.createElement("span");
  running.className = "meta";
  running.textContent = "running";
  acts.append(running);
}