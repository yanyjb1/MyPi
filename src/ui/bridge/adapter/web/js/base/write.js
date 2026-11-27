// Everything the page tells the server, and the four rows of a conversation's own header.

import { post, subscribe } from "./wire.js";
import { announce, failed, state } from "./state.js";
import { open, pools, refresh } from "./read.js";
import { entry } from "./model.js";

// Safe model split at first colon only, preserving model IDs containing colons (e.g., provider:model:tag)
function parseModel(addr) {
  const at = (addr || "").indexOf(":");
  if (at < 0) return { provider: "", model: addr || "" };
  return { provider: addr.slice(0, at), model: addr.slice(at + 1) };
}

// One conversation made on the server, then entered: a draft becomes a session here and nowhere else.
export async function create(where, model, effort) {
  try {
    const ask = { cwd: where || "~", fallback: !where, effort };
    if (model) ask.model = parseModel(model);
    const answer = await post("init", ask);
    state.draft = null;
    await refresh();
    return answer.session;
  } catch (e) {
    failed(e);
  }
}

export async function remove(id) {
  try {
    await post("delete", { session: id });
    if (state.active === id) {
      state.active = null;
      state.session = null;
    }
    state.byId.delete(id);
    await refresh();
  } catch (e) {
    failed(e);
  }
}

export function interrupt() {
  const held = state.session;
  if (!held) return;
  post("interrupt", { session: held.id }).catch(failed);
}

// The three writes land on this conversation's own row; the signal skips the writer, so this side updates it.
function header(name, content, onto) {
  const held = state.session;
  if (!held) return Promise.resolve(false);
  // `session` sits at the top so the door knows which connection to carry this on: the one holding the conversation.
  return post("meta", {
    name,
    session: held.id,
    content: { session: held.id, ...content },
  })
    .then(() => {
      held.meta = { ...held.meta, ...onto };
      announce();
      if (name !== "effort") refresh();
      return true;
    })
    .catch((e) => {
      failed(e);
      return false;
    });
}

// A row's name is written on that row: the one on screen, or another the rail names.
export function rename(name, id) {
  const held = state.session;
  if (!id || (held && held.id === id)) {
    header("name", { name: name || null }, { name: name || null });
    return;
  }
  post("meta", {
    name: "name",
    session: id,
    content: { session: id, name: name || null },
  })
    .then(() => refresh())
    .catch(failed);
}

// The directory moves; its landed form is the server's answer, so the row is asked for again.
export function move(path) {
  return header("cwd", { path }, {})
    .then((ok) => (ok ? open(state.active).then(() => true) : false))
    .catch(() => false);
}

export async function switchModel(address) {
  const held = state.session;
  if (!held) return;
  const { provider, model } = parseModel(address);
  try {
    await entry(address);
    await post("meta", {
      name: "switch",
      session: held.id,
      content: { session: held.id, model: { provider, model } },
    });
    held.meta = { ...held.meta, provider, model };
    held.effort = "auto";
    announce();
  } catch (e) {
    failed(e);
  }
}

export function setRung(rung) {
  if (state.session) {
    state.session.effort = rung;
    if (state.session.meta) state.session.meta.effort = rung;
    header("effort", { effort: rung }, { effort: rung });
  } else if (state.draft) {
    state.draft.effort = rung;
    announce();
  }
}

export function defaultModel(address) {
  const { provider, model } = parseModel(address);
  post("default_model", { model: { provider, model } })
    .then(() => pools())
    .catch(failed);
}


export function compact(focus) {
  const held = state.session;
  if (!held) {
    failed(new Error("/compact needs a session"));
    return;
  }
  const payload = { session: held.id };
  if (focus && focus.trim()) payload.focus = focus.trim();
  subscribe("compact", payload).catch(failed);
}

