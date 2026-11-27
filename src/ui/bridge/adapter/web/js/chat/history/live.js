// Every frame the bridge pushes, folded into one conversation's own memory.

import { said } from "../../base/note.js";
import { announce, state } from "../../base/state.js";
import { bill, blocks, open, refresh } from "../../base/read.js";

export function absorb(held, frame) {
  if (frame.kind !== "stream") return;
  const payload = frame.payload || {};
  if (payload.session != null && payload.session !== held.id) return;
  // The round this stream is saying: its landed copy is not drawn beside it.
  if (payload.turn != null) held.liveTurn = payload.turn;
  if (frame.name === "notice") {
    held.live.push({
      type: "notice",
      level: (payload.text && payload.text.kind) || "notice",
      text: said(payload.text),
    });
    announce("tail");
    return;
  }
  if (frame.name === "signal") {
    if (payload.signal === "rewind") open(held.id);
    else if (payload.signal === "name") refresh();
    else if (payload.signal === "model" || payload.signal === "cwd") {
      open(held.id);
    } else if (payload.signal === "user" || payload.signal === "mailbox") {
      // The api treats the writer as the issuer, so this page's own letter comes back wordless: its words are the queue's.
      if (payload.text) echo(held, payload.text);
      else {
        // A queued line opening takes one line off the composer's queue: the composer moves with it.
        const opened = held.queued.shift();
        if (opened) {
          held.live.push({ type: "user", text: opened });
          announce();
        }
      }
    }
    return;
  }
  // A lap's bill rides the last piece it wrote, and a zeroed one is a fact the provider states rather than a
  // bill: a quote lands once a lap, and that is when the drawer's figures and the ring are due again.
  if (payload.usage && payload.usage.prompt > 0) bill(held.id);
  // The round closed: its rows are blocks now, so the composer that showed it running lets go too.
  if (payload.ranges) {
    closed(held, payload);
    state.streaming = false;
    announce();
    return;
  }
  // The first piece of a round is not only the tail: it arms the composer's stop too.
  announce(piece(held, payload) ? "all" : "tail");
}

// A message another front end sent into this conversation: a user line of its own from the first frame.
function echo(held, text) {
  if (!text) return;
  held.live.push({ type: "user", text: String(text) });
  // A whole page, not the tail: the round this line belongs to may already be laid below it, and the
  // page is what takes that copy away.
  announce();
}

// A round starting over: no live piece of the last one is still in hand.
export function forget() {
  calls.clear();
}

// One live piece: text grows its own row, thinking grows the last one, calls grow by id. Whether this piece
// opened the round is what the caller says the page with.
function piece(held, payload) {
  const started = !held.busy;
  held.busy = true;
  state.streaming = true;
  const last = held.live[held.live.length - 1];
  // A running text is laid a piece at a time, so a carriage return is folded away here, once, rather than
  // wherever the whole of it is read again.
  const text = String(payload.text || "").replace(/\r\n?/g, "\n");
  switch (payload.kind) {
    case "text":
      if (last && last.type === "text" && last.open) last.text += text;
      else held.live.push({ type: "text", text, open: true });
      break;
    case "thinking":
      if (last && last.type === "thinking") last.text += text;
      else held.live.push({ type: "thinking", text });
      break;
    case "boundary":
      held.live.push({ type: "break" });
      break;
    case "wake":
      held.live.push({ type: "wake", text });
      break;
    case "arguments": {
      const at = byCall(held, payload.call_id);
      if (at) {
        at.args = (at.args || "") + text;
        at.rev += 1;
      }
      break;
    }
    case "tool_start": {
      let args = payload.args || "";
      try {
        args = JSON.stringify(JSON.parse(args), null, 2);
      } catch {
        // A half-said argument list is shown as it arrived.
      }
      const item = {
        type: "tool",
        call_id: payload.call_id,
        name: payload.name || "",
        args,
        intent: payload.intent || "",
        status: "running",
        note: "",
        rev: 1,
      };
      held.live.push(item);
      calls.set(held.id + ":" + payload.call_id, item);
      break;
    }
    case "tool_progress": {
      const at = byCall(held, payload.call_id);
      if (at) {
        at.note = text;
        at.rev += 1;
      }
      break;
    }
    case "tool_end": {
      const at = byCall(held, payload.call_id);
      const items = Array.isArray(payload.ok) ? payload.ok : [];
      const failed = items.some((item) => item && item.ok === false);
      if (at) {
        at.status = failed ? "failed" : "done";
        at.result = payload.result || "";
        at.took = payload.duration_ms || 0;
        at.note = "";
        at.rev += 1;
      }
      break;
    }
    default:
      break;
  }
  return started;
}

// The one live call this id names: a round says hundreds of argument pieces, so the lookup is kept, not walked.
// A call belongs to the conversation that said it, and two conversations may name their calls alike.
const calls = new Map();

function byCall(held, id) {
  return calls.get(held.id + ":" + id) || null;
}

// The round closed: its rows come from storage now, so the live pieces go and the ranges are asked for.
function closed(held, payload) {
  held.busy = false;
  held.liveTurn = null;
  held.live = [];
  const round = {
    turn: payload.turn,
    ranges: payload.ranges,
    compact: !!payload.compact,
  };
  held.liveNode = null;
  calls.clear();
  const at = held.structure.findIndex((one) => one.turn === round.turn);
  if (at >= 0) held.structure[at] = round;
  else held.structure.push(round);
  blocks(held.id, round.ranges);
}
