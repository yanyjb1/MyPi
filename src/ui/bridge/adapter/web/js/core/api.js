// The door: the one socket that carries every question and the one conversation followed, and every read and write
// the page makes of the server — each answer landing in the store as the page's own state.

import { address, announce, ensure, failed, keep, state } from "./store.js";

// The name the page answers to; a browser may not set a head on a socket, so the name is said as its sub-protocol.
const ADAPTER = "webui";

// The ladder a broken socket is dialled again on, how long a dial waits for the handshake, and how long a question
// waits for its answer before the door is called silent.
const LATER = [500, 1000, 2000, 4000, 8000];
const HANDSHAKE = 5000;
const PATIENCE = 35_000;

// The socket, the dial in flight, and the turn a question's id is taken from.
let link = null;
let dialing = null;
let seq = 0;
// Questions waiting for an answer, by the id the door echoes back.
const waiting = new Map();
// The conversation this page follows, and where its frames go: both outlive a broken socket.
let watching = null;
let sink = null;
// The open socket's own end, as a promise whoever follows a conversation waits on.
let ended = Promise.resolve();
// The door is on its way shut: a connection is what keeps a conversation alive, so it goes once nothing wants it.
let closing = false;

// The server refuses with a keyed text and the door with a code; a stream notice carries that same shape, so this is
// also how a notice becomes a line.
export function reason(bad) {
  if (!bad) return "refused";
  if (bad.text) return bad.text;
  if (bad.key) {
    const args = bad.args && Object.keys(bad.args).length ? " " + JSON.stringify(bad.args) : "";
    return bad.key + args;
  }
  return bad.message || bad.code || "refused";
}

class Refused extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

// One frame, one answer: the payload the server wrote, or the refusal as it came.
async function ask(kind, name, payload = {}) {
  const ws = await reach();
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      waiting.delete(id);
      reject(new Refused("silent", `no answer to ${kind}/${name}`));
      settle();
    }, PATIENCE);
    waiting.set(id, { resolve, reject, timer });
    try {
      ws.send(JSON.stringify({ id, kind, name, payload }));
    } catch (e) {
      waiting.delete(id);
      clearTimeout(timer);
      reject(new Refused("unreachable", `the bridge is not answering: ${e.message}`));
    }
  });
}

// The one socket, dialled once however many ask for it at the same moment.
function reach() {
  if (link && link.readyState === WebSocket.OPEN) return Promise.resolve(link);
  if (dialing) return dialing;
  closing = false;
  dialing = dial().finally(() => {
    dialing = null;
  });
  return dialing;
}

function dial() {
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return new Promise((resolve, reject) => {
    let ws;
    try {
      ws = new WebSocket(`${scheme}//${location.host}/`, ADAPTER);
    } catch (e) {
      reject(new Refused("unreachable", `the bridge is not answering: ${e.message}`));
      return;
    }
    let settled = false;
    const giveUp = (said) => {
      if (settled) return;
      settled = true;
      clearTimeout(late);
      reject(new Refused("unreachable", said));
    };
    const late = setTimeout(() => {
      giveUp("the bridge did not answer the socket");
      ws.close();
    }, HANDSHAKE);
    ws.addEventListener("open", () => {
      if (settled) return;
      settled = true;
      clearTimeout(late);
      link = ws;
      attend(ws);
      resolve(ws);
    });
    ws.addEventListener("error", () => giveUp("the bridge closed the socket"));
    ws.addEventListener("close", () => giveUp("the bridge closed the socket"));
  });
}

// Nothing follows a conversation and nothing waits on a reply any more: the socket goes, and with it the reason the
// server had to keep that conversation. The check waits one turn of the event loop, so a page moving from one
// conversation to the next keeps the socket it already has rather than shaking hands again.
function shut() {
  closing = true;
  setTimeout(settle, 0);
}

function settle() {
  if (!closing || watching !== null || waiting.size) return;
  closing = false;
  const ws = link;
  link = null;
  if (ws) ws.close();
}

// Every frame the socket says: one carrying an id on the ledger answers that question and nothing else, and the rest
// belong to the conversation this page follows. A reply whose question was given up on carries an id nobody holds.
function attend(ws) {
  ended = new Promise((done) => ws.addEventListener("close", () => done(), { once: true }));
  ws.addEventListener("message", (event) => {
    let frame;
    try {
      frame = JSON.parse(event.data);
    } catch {
      return;
    }
    if (!frame || typeof frame !== "object") return;
    if (frame.id != null && waiting.has(frame.id)) {
      const asked = waiting.get(frame.id);
      waiting.delete(frame.id);
      clearTimeout(asked.timer);
      // The id is the door's own field, so the payload handed on carries none of it.
      delete frame.id;
      if (frame.error) {
        asked.reject(new Refused(frame.error.code || "refused", reason(frame.error)));
      } else {
        asked.resolve(frame);
      }
      settle();
      return;
    }
    if (sink) sink(frame);
  });
  ws.addEventListener("close", () => {
    if (link === ws) link = null;
    for (const [id, asked] of waiting) {
      clearTimeout(asked.timer);
      asked.reject(new Refused("unreachable", "the bridge closed the socket"));
      waiting.delete(id);
    }
  });
}

const call = (name, payload) => ask("call", name, payload);
const post = (name, payload) => ask("post", name, payload);
const subscribe = (name, payload) => ask("subscribe", name, payload);
const local = (name, payload) => ask("local", name, payload);

// The two writes a message makes and the one read a jump list makes, named for what they say rather than for the
// door they go through.
export const mailbox = (payload) => post("mailbox", payload);
export const opening = (payload) => subscribe("submit", payload);
export const words = (payload) => call("blocks", payload);

// The notes: one door reads them, the other writes them.
export const memosRead = (ask) => call("memo", ask);
export const memosWrite = (ask) => post("memo", ask);

// One conversation's frames. The door tells a follower everything the running round has said so far the moment it is
// told to follow, so a broken socket or a reloaded page loses nothing; `again` is what the page clears in exchange, and
// it runs right after the watch is sent, because that replay may land before the watch's own reply does.
export function follow(session, handlers) {
  let closed = false;
  let rung = 0;

  const stop = () => {
    closed = true;
    sink = null;
    if (watching === session) watching = null;
    shut();
  };

  const run = async () => {
    while (!closed) {
      try {
        const ws = await reach();
        if (closed) return;
        watching = session;
        closing = false;
        sink = handlers.frame;
        ws.send(
          JSON.stringify({
            kind: "subscribe",
            name: "watch",
            payload: { session },
          }),
        );
        rung = 0;
        handlers.again();
        await ended;
      } catch {
        if (closed) return;
      }
      if (closed) return;
      const wait = LATER[Math.min(rung, LATER.length - 1)];
      rung += 1;
      await new Promise((done) => setTimeout(done, wait));
    }
  };

  run();
  return { close: stop };
}

// The session list, anchor rows left out: a session's anchor is not a session.
export async function refresh() {
  const before = fingerprint(state.rows);
  let changed = false;
  try {
    const answer = await call("sessions");
    const rows = (answer.rows || []).filter((row) => !row.anchor);
    changed = fingerprint(rows) !== before;
    state.rows = rows;
    if (state.rows.length && !state.active) {
      state.active = state.rows[0].id;
      changed = true;
    }
  } catch (e) {
    failed(e);
  }
  // A list nobody changed is not a page's worth of redraw: the rail polls this every few seconds.
  if (changed) announce();
}

// What a list of rows says, as one string to compare.
function fingerprint(rows) {
  return JSON.stringify(rows || []);
}

// One conversation's rows: its structure, the blocks it names, and the rungs it runs at.
export async function open(id) {
  const held = ensure(id);
  try {
    const [snapshot, meta] = await Promise.all([call("init", { session: id }), call("meta", { session: id })]);
    held.meta = meta;
    held.structure = snapshot.structure || [];
    held.blocks = new Map();
    held.view.clear();
    held.heads = null;
    held.span = null;
    for (const block of snapshot.blocks || []) held.blocks.set(block.id, block);
    held.effort = snapshot.effort || "auto";
  } catch (e) {
    failed(e);
  }
  announce();
}

// Ranges asked for and not yet answered: two questions about the same blocks are one question.
const pendingRanges = new Set();

// Bodies for the ranges asked for: deduped in flight, and a round that meets a fetched one is laid again.
export async function blocks(id, ranges) {
  const held = ensure(id);
  const uncached = (ranges || []).filter(([lo, hi]) => {
    const key = id + ":" + lo + "-" + hi;
    if (pendingRanges.has(key)) return false;
    pendingRanges.add(key);
    return true;
  });

  if (!uncached.length) return;

  try {
    const answer = await call("blocks", { session: id, ranges: uncached });
    for (const block of answer.blocks || []) held.blocks.set(block.id, block);
    // A round whose own ranges meet a fetched one is laid again: it is found by sweeping the rounds once, rather
    // than once per range.
    for (const round of held.structure) {
      if (uncached.some(([lo, hi]) => meets(round.ranges, lo, hi))) {
        held.view.delete(round.turn);
      }
    }
    announce();
  } catch (e) {
    failed(e);
  } finally {
    for (const [lo, hi] of uncached) {
      pendingRanges.delete(id + ":" + lo + "-" + hi);
    }
  }
}

// Whether any of a round's own ranges meets the one just fetched.
function meets(ranges, lo, hi) {
  return (ranges || []).some(([start, end]) => start <= hi && lo <= end);
}

// A bill asked for while an earlier one is still on its way: the answer in flight settles both, since a lap's quote
// arrives far faster than the ledger takes to read.
const bills = new Map();

export async function bill(id) {
  const at = id || state.active;
  if (!at) return;
  const flying = bills.get(at);
  if (flying) return flying;
  const asked = call("usage", { session: at })
    .then((usage) => {
      ensure(at).usage = usage;
      announce("bill");
    })
    .catch(failed)
    .finally(() => bills.delete(at));
  bills.set(at, asked);
  return asked;
}

export async function pools(where) {
  try {
    state.pools = await call("pools", where ? { cwd: where } : {});
    announce();
  } catch (e) {
    failed(e);
  }
}

// What one directory holds: the completer's and the draft's one look at the machine.
export function listing(where, path) {
  const payload = {};
  if (where) payload.cwd = where;
  if (path) payload.path = path;
  return local("list", payload);
}

// What the pages a round cites call themselves: the door reads them, since a page may not read another site's head.
export function citeTitles(urls) {
  return local("title", { urls });
}

// Safe model split at the first colon only, so an address naming a tag of its own keeps it.
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
      keep(null);
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
  // `session` sits at the top so the door knows which connection carries this: the one holding the conversation.
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

// The model table this page reads for context size and thinking rungs, and the one address it asks about.
const entries = new Map();

// The model a draft would open with when the page has none of its own: the server's default.
export async function fallbackModel() {
  try {
    const answer = await call("config", {
      key: "config.server.api.defaultModel",
    });
    if (answer.value) return String(answer.value);
  } catch {}
  return null;
}

export async function entry(address) {
  if (!address) return null;
  if (entries.has(address)) return entries.get(address);
  try {
    const answer = await call("config", { key: "models.model." + address });
    entries.set(address, answer.value || null);
  } catch {
    entries.set(address, null);
  }
  announce();
  return entries.get(address);
}

export function contextWindow() {
  const found = address() && entries.get(address());
  return (found && found.contextWindow) || 0;
}

export async function windowed() {
  await entry(address());
}

export function rungs() {
  const found = address() && entries.get(address());
  const list = (found && found.reasoningEfforts) || [];
  return ["auto"].concat(list);
}
