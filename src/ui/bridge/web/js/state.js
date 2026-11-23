// What the page knows: the rail, one conversation's rounds, the panel, and every frame folded in.

import { call, post, subscribe, follow, Refused } from "./wire.js";

const watchers = new Set();

// The page state. `session` is the conversation on screen; `byId` keeps every one visited.
export const state = {
	rows: [],
	active: null,
	session: null,
	panel: { open: false, tab: "usage" },
	pools: null,
	refusal: null,
	streaming: false,
	byId: new Map(),
};

export function watch(fn) {
	watchers.add(fn);
	fn();
}

export function announce() {
	for (const fn of watchers) fn();
}

function blank(id) {
	return {
		id,
		meta: null,
		structure: [],
		blocks: new Map(),
		live: [],
		busy: false,
		effort: "auto",
		usage: null,
		todo: [],
		memos: null,
		read: null,
		children: null,
		queued: [],
	};
}

function said(text) {
	if (!text) return "";
	if (typeof text === "string") return text;
	if (text.text) return text.text;
	if (text.key) {
		const args = text.args && Object.keys(text.args).length ? " " + JSON.stringify(text.args) : "";
		return text.key + args;
	}
	return text.message || "";
}

// One refusal, said in the transcript; Escape closes the panel and nothing else.
export function failed(e) {
	state.refusal = e instanceof Refused ? e.message : String(e);
	announce();
}

export function dismiss() {
	state.refusal = null;
	announce();
}

// ─── the panel ───────────────────────────────────────────────────────────

export function openPanel(at) {
	state.panel.open = true;
	if (at) state.panel.tab = at;
	announce();
}

export function closePanel() {
	state.panel.open = false;
	announce();
}

export function tab(name) {
	state.panel.open = true;
	state.panel.tab = name;
	announce();
}

// The panels a line of the composer may summon, each with the write it stands for.
const SUMMON = {
	usage: () => {
		openPanel("usage");
		bill();
	},
	todo: () => {
		openPanel("todo");
		todos();
	},
	tree: () => openPanel("tree"),
	child: () => {
		openPanel("child");
		kids();
	},
	children: () => {
		openPanel("child");
		kids();
	},
	memo: (word) => {
		openPanel("memo");
		memos(word);
	},
	new: (word) => create(word || undefined),
	cdp: (word) => {
		openPanel("setup");
		if (word) move(word);
	},
	setup: () => {
		openPanel("setup");
		pools();
	},
	model: () => {
		openPanel("setup");
		pools();
	},
	switch: (word) => {
		openPanel("setup");
		switchWord(word);
	},
	effort: (word) => {
		openPanel("setup");
		if (word) setEffort(word);
	},
	name: (word) => {
		openPanel("setup");
		if (word) rename(word);
	},
	cwd: (word) => {
		openPanel("setup");
		if (word) move(word);
	},
	compact: (word) => {
		openPanel("setup");
		compact(word);
	},
};

// `provider:model`, the one form a switch takes.
function switchWord(word) {
	const [provider, model] = String(word || "").split(":");
	if (!provider || !model) return;
	switchModel(provider, model);
}

export function summon(line) {
	const text = String(line || "").trim();
	if (!text.startsWith("/")) return false;
	const [head, ...rest] = text.slice(1).split(/\s+/);
	const found = SUMMON[head];
	if (!found) return false;
	found(rest.join(" ").trim());
	return true;
}

// ─── the rail ────────────────────────────────────────────────────────────

export async function refresh() {
	try {
		const answer = await call("sessions");
		state.rows = (answer.rows || []).filter((row) => !row.anchor);
		if (state.rows.length && !state.active) state.active = state.rows[0].id;
	} catch (e) {
		failed(e);
	}
	announce();
}

export async function create(where, model) {
	try {
		const ask = { cwd: where || "~", fallback: !where, model: model || undefined };
		const answer = await post("init", ask);
		await refresh();
		await enter(answer.session);
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
		await refresh();
	} catch (e) {
		failed(e);
	}
}

// ─── one conversation ────────────────────────────────────────────────────

let streaming = null;

export async function enter(id) {
	if (!id) return;
	if (streaming) {
		streaming.close();
		streaming = null;
	}
	state.active = id;
	state.refusal = null;
	let held = state.byId.get(id);
	if (!held) {
		held = blank(id);
		state.byId.set(id, held);
	}
	state.session = held;
	announce();
	streaming = follow(id, (frame) => absorb(held, frame));
	try {
		const [snapshot, meta] = await Promise.all([call("init", { session: id }), call("meta", { session: id })]);
		held.meta = meta;
		held.structure = snapshot.structure || [];
		held.blocks = new Map();
		for (const block of snapshot.blocks || []) held.blocks.set(block.id, block);
		held.todo = snapshot.todo || [];
		held.effort = snapshot.effort || "auto";
		held.busy = false;
		held.live = [];
		state.streaming = false;
		announce();
		await Promise.all([bill(id), windowed()]);
	} catch (e) {
		failed(e);
	}
}

export async function bill(id) {
	const at = id || state.active;
	if (!at) return;
	try {
		const usage = await call("usage", { session: at });
		const held = state.byId.get(at);
		if (held) held.usage = usage;
		announce();
	} catch (e) {
		failed(e);
	}
}

export async function todos(id) {
	const at = id || state.active;
	if (!at) return;
	try {
		const list = await call("todo", { session: at });
		const held = state.byId.get(at);
		if (held) held.todo = list || [];
		announce();
	} catch (e) {
		failed(e);
	}
}

export async function memos(word) {
	const id = state.active;
	if (!id) return;
	try {
		const answer = await call("memo", { session: id, op: "list" });
		const held = state.byId.get(id);
		if (held) held.memos = answer.memos || [];
		announce();
		if (word) readWord(word);
	} catch (e) {
		failed(e);
	}
}

// A word after `/memo`: an op, a scope, and a name.
function readWord(word) {
	const [op, ...rest] = String(word || "").split(/\s+/);
	const scope = rest[0] === "current" || rest[0] === "global" ? rest.shift() : "global";
	const name = rest.join(" ").trim();
	if (!name) return;
	if (op === "history" || op === "del") readMemo(scope, name);
}

export async function readMemo(scope, name) {
	try {
		const answer = await call("memo", { session: scope === "current" ? state.active : undefined, op: "history", scope, name });
		const held = state.byId.get(state.active);
		if (held) held.read = { scope, name, versions: answer.versions || [] };
		announce();
	} catch (e) {
		failed(e);
	}
}

export function closeMemo() {
	const held = state.byId.get(state.active);
	if (held) held.read = null;
	announce();
}

export async function writeMemo(op, scope, name, content) {
	try {
		await post("memo", { session: scope === "current" ? state.active : undefined, op, scope, name, content });
		const held = state.byId.get(state.active);
		if (held) held.read = null;
		await memos();
	} catch (e) {
		failed(e);
	}
}

export async function kids() {
	const id = state.active;
	if (!id) return;
	try {
		const answer = await call("children", { session: id });
		const held = state.byId.get(id);
		if (held) held.children = answer.rows || [];
		announce();
	} catch (e) {
		failed(e);
	}
}

export async function pools(where) {
	try {
		state.pools = await call("pools", where ? { cwd: where } : {});
		announce();
	} catch (e) {
		failed(e);
	}
}

// The context window of the model a session runs, asked once and kept.
const windows = new Map();

export async function windowed() {
	const model = address();
	if (!model || windows.has(model)) return;
	try {
		const answer = await call("config", { key: "models.model." + model + ".contextWindow" });
		windows.set(model, Number(answer.value) || 0);
		announce();
	} catch {
		windows.set(model, 0);
	}
}

export function window() {
	return windows.get(address()) || 0;
}

// `provider:model`, the one address a pool entry and a cost row both answer to.
function address() {
	const meta = state.session && state.session.meta;
	if (!meta || !meta.model) return "";
	return (meta.provider ? meta.provider + ":" : "") + meta.model;
}

// ─── words in ────────────────────────────────────────────────────────────

export function send(text, prefix) {
	const body = String(text || "");
	if (!body.trim()) return;
	const held = state.session;
	if (!held) {
		create().then(() => send(body, prefix));
		return;
	}
	const payload = { session: held.id, text: body };
	if (prefix) payload.prefix = prefix;
	const running = held.busy;
	held.busy = true;
	if (running) held.queued.push(body);
	announce();
	const quiet = running ? post("mailbox", payload) : subscribe("submit", payload);
	quiet.catch((e) => {
		if (running) {
			held.queued = held.queued.filter((one) => one !== body);
			held.busy = held.queued.length > 0;
			announce();
		}
		failed(e);
	});
}

export function interrupt() {
	const held = state.session;
	if (!held) return;
	post("interrupt", { session: held.id }).catch(failed);
}

// The three writes land on this session's own row; the signal skips the writer, so this side updates it.
export function rename(name) {
	const held = state.session;
	if (!held) return;
	post("meta", { name: "name", content: { session: held.id, name: name || null } })
		.then(() => {
			held.meta = { ...held.meta, name: name || null };
			announce();
			refresh();
		})
		.catch(failed);
}

export function move(path) {
	const held = state.session;
	if (!held) return;
	post("meta", { name: "cwd", content: { session: held.id, path } })
		.then(() => {
			held.meta = { ...held.meta, cwd: path };
			announce();
			refresh();
		})
		.catch(failed);
}

export function switchModel(provider, model) {
	const held = state.session;
	if (!held) return;
	post("meta", { name: "switch", content: { session: held.id, model: { provider, model } } })
		.then(() => {
			held.meta = { ...held.meta, provider, model };
			held.effort = "auto";
			announce();
			bill(held.id);
			windowed();
		})
		.catch(failed);
}

export function setEffort(effort) {
	const held = state.session;
	if (!held) return;
	post("meta", { name: "effort", content: { session: held.id, effort } })
		.then(() => {
			held.effort = effort;
			announce();
		})
		.catch(failed);
}

export function defaultModel(provider, model) {
	post("default_model", { model: { provider, model } })
		.then(() => pools())
		.catch(failed);
}

export function compact(focus) {
	const held = state.session;
	if (!held) return;
	const payload = { session: held.id };
	if (focus && focus.trim()) payload.focus = focus.trim();
	subscribe("compact", payload).catch(failed);
}

export function rewind(turn, block) {
	const held = state.session;
	if (!held) return;
	post("meta", { name: "rewind", content: { session: held.id, turn, block } })
		.then(() => enter(held.id))
		.catch(failed);
}

// ─── frames in ───────────────────────────────────────────────────────────

function absorb(held, frame) {
	if (frame.kind !== "stream") return;
	const payload = frame.payload || {};
	if (payload.session != null && payload.session !== held.id) return;
	if (frame.name === "notice") {
		held.live.push({ type: "notice", level: (payload.text && payload.text.kind) || "notice", text: said(payload.text) });
		announce();
		return;
	}
	if (frame.name === "signal") {
		if (payload.signal === "rewind") enter(held.id);
		else if (payload.signal === "usage" || payload.signal === "model" || payload.signal === "effort") bill(held.id);
		else if (payload.signal === "name") refresh();
		else if (payload.signal === "user" || payload.signal === "mailbox") {
			if (payload.text) echo(held, payload.text);
			else held.queued.shift();
			announce();
		}
		return;
	}
	if (payload.ranges) {
		closed(held, payload);
		return;
	}
	piece(held, payload);
	announce();
}

// A message another front end sent into this conversation, kept as a live row.
function echo(held, text) {
	if (!text) return;
	held.live.push({ type: "text", text: String(text) });
	announce();
}

// One live piece: text and thinking grow their last piece, calls grow by id.
function piece(held, payload) {
	held.busy = true;
	state.streaming = true;
	const last = held.live[held.live.length - 1];
	const text = payload.text || "";
	switch (payload.kind) {
		case "text":
			held.live.push({ type: "text", text });
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
			if (at) at.args = (at.args || "") + text;
			break;
		}
		case "tool_start": {
			let args = payload.args || "";
			try {
				args = JSON.stringify(JSON.parse(args), null, 2);
			} catch {
				// A half-said argument list is shown as it arrived.
			}
			held.live.push({
				type: "tool",
				call_id: payload.call_id,
				name: payload.name || "",
				args,
				intent: payload.intent || "",
				status: "running",
				note: "",
			});
			break;
		}
		case "tool_progress": {
			const at = byCall(held, payload.call_id);
			if (at) at.note = text;
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
			}
			break;
		}
		default:
			break;
	}
}

function byCall(held, id) {
	for (let at = held.live.length - 1; at >= 0; at -= 1) {
		const item = held.live[at];
		if (item.type === "tool" && item.call_id === id) return item;
	}
	return null;
}

// The round closed: its rows come from storage now, so the live pieces go and the ranges are asked for.
function closed(held, payload) {
	held.busy = false;
	state.streaming = false;
	held.live = [];
	const round = { turn: payload.turn, ranges: payload.ranges, compact: !!payload.compact };
	const at = held.structure.findIndex((one) => one.turn === round.turn);
	if (at >= 0) held.structure[at] = round;
	else held.structure.push(round);
	announce();
	call("blocks", { session: held.id, ranges: round.ranges })
		.then((answer) => {
			for (const block of answer.blocks || []) held.blocks.set(block.id, block);
			announce();
			bill(held.id);
		})
		.catch(failed);
}

// ─── reading the model ───────────────────────────────────────────────────

// The blocks of one round, oldest first, with each tool result paired to its request.
export function roundRows(blocks) {
	const requests = new Map();
	for (const block of blocks) {
		if (block.kind === "tool_request" && block.call_id) requests.set(block.call_id, block);
	}
	return blocks.map((block) => {
		if (block.kind === "tool_result" && block.call_id && requests.has(block.call_id)) {
			return { block, paired: requests.get(block.call_id) };
		}
		return { block, paired: null };
	});
}

export function blocksOf(held, round) {
	const found = [];
	for (const [lo, hi] of round.ranges || []) {
		for (const block of held.blocks.values()) {
			if (block.id >= lo && block.id <= hi) found.push(block);
		}
	}
	found.sort((a, b) => a.id - b.id);
	return roundRows(found);
}

export { said };