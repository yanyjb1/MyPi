// The page's one render: the rail, the transcript, and the panel, all from the state.

import { body, note, short, shorten, tokens, money, elapsed, resultText } from "./text.js";
import * as page from "./state.js";

const state = page.state;

const el = (tag, props, ...kids) => {
	const node = document.createElement(tag);
	for (const [key, value] of Object.entries(props || {})) {
		if (value == null || value === false) continue;
		if (key === "class") node.className = value;
		else if (key === "text") node.textContent = value;
		else if (key === "html") node.innerHTML = value;
		else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
		else node.setAttribute(key, value);
	}
	for (const kid of kids.flat()) {
		if (kid == null || kid === false) continue;
		node.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
	}
	return node;
};

const at = (id) => document.getElementById(id);
const clear = (node) => {
	while (node.firstChild) node.removeChild(node.firstChild);
	return node;
};

// ─── the rail ────────────────────────────────────────────────────────────

function rowTitle(row) {
	if (row.name) return row.name;
	if (row.first_user) return short(row.first_user, 80);
	return "session " + row.id;
}

function rail() {
	const list = clear(at("rows"));
	at("count").textContent = state.rows.length ? String(state.rows.length) : "";
	if (!state.rows.length) {
		list.append(el("li", { class: "empty", text: "no sessions yet" }));
		return;
	}
	for (const row of state.rows) {
		const running = state.session && state.session.id === row.id && state.session.busy;
		list.append(
			el(
				"li",
				{ "data-on": row.id === state.active ? "1" : null, onclick: () => page.enter(row.id) },
				el("span", { class: "title", text: rowTitle(row) }),
				el(
					"span",
					{ class: "sub" },
					el("span", { text: "#" + row.id }),
					row.rounds ? el("span", { text: row.rounds + "r" }) : null,
					row.last_ts ? el("span", { text: short(String(row.last_ts).replace("T", " "), 16) }) : null,
					running ? el("span", { class: "live", text: "running" }) : null,
				),
			),
		);
	}
}

// ─── the top bar ─────────────────────────────────────────────────────────

function topbar() {
	const held = state.session;
	const title = clear(at("title"));
	const acts = clear(at("session-acts"));
	at("main").dataset.empty = held ? "0" : "1";
	if (!held) {
		title.append(el("h1", { text: "mypi" }));
		at("panel-toggle").hidden = true;
		return;
	}
	at("panel-toggle").hidden = false;
	const meta = held.meta || {};
	title.append(
		el("h1", { text: meta.name || "session " + held.id }),
		el("span", { class: "where", title: meta.cwd || "", text: shorten(meta.cwd || "", 48) }),
	);
	if (held.busy) acts.append(el("span", { class: "chip", "data-live": "1", text: "running" }));
	if (meta.provider && meta.model) acts.append(el("span", { class: "chip", text: meta.provider + ":" + meta.model }));
	if (held.effort && held.effort !== "auto") acts.append(el("span", { class: "chip", text: held.effort }));
}

// ─── one block ───────────────────────────────────────────────────────────

function you(block) {
	const kids = [];
	if (block.prefix) kids.push(el("span", { class: "wrap", text: block.prefix }));
	kids.push(el("span", { text: block.content || block.text || "" }));
	if (block.suffix) kids.push(el("span", { class: "wrap", text: block.suffix }));
	return el("div", { class: "you" }, kids);
}

function assistant(text) {
	return el("div", { class: "md", html: body(text || "") });
}

function reasoning(text) {
	const lines = String(text || "").split("\n").length;
	return el(
		"details",
		{ class: "think", open: state.streaming },
		el("summary", { text: "thinking · " + lines + (lines === 1 ? " line" : " lines") }),
		el("div", { class: "body md", html: body(text || "") }),
	);
}

function wake(text) {
	return el(
		"div",
		{ class: "note", "data-level": "notice" },
		el("span", { class: "src", text: "wake" }),
		el("span", { text: text || "" }),
	);
}

// One tool call: the head is name, one line of what it is doing, its state and its cost.
function toolView(name, args, intent, status, result, took, progress) {
	const node = el("details", { class: "tool", "data-state": status || "running" });
	if (status !== "running") node.setAttribute("open", "");
	const head = el(
		"summary",
		{},
		el("span", { class: "dot" }),
		el("span", { class: "name", text: name || "call" }),
		el("span", { class: "what", text: progress || note(name, args, intent) }),
		el(
			"span",
			{ class: "tail" },
			el("span", { text: status === "running" ? "running" : status === "failed" ? "error" : "done" }),
			took ? el("span", { text: elapsed(took) }) : null,
		),
	);
	const inside = el("div", { class: "inside" });
	const pairs = args && typeof args === "object" ? Object.entries(args) : [];
	if (pairs.length) {
		inside.append(
			el(
				"div",
				{ class: "args" },
				pairs.map(([key, value]) => {
					const flat = typeof value === "string" ? value : JSON.stringify(value);
					return [el("span", { class: "k", text: key }), el("span", { class: "v", title: flat, text: short(flat, 140) })];
				}),
			),
		);
	}
	const text = result ? resultText(result) : "";
	if (text) inside.append(el("pre", { class: "out", "data-tone": status === "failed" ? "failed" : null, text }));
	node.append(head, inside);
	return node;
}

function parsed(raw) {
	if (!raw) return {};
	if (typeof raw === "object") return raw;
	try {
		return JSON.parse(raw);
	} catch {
		return { raw: String(raw) };
	}
}

// One landed block: a request with its result, or one of the message bodies.
function blockView(block, paired) {
	switch (block.kind) {
		case "user":
			return you(block);
		case "wake":
			return wake(block.text || block.content);
		case "assistant":
			return assistant(block.content || block.text);
		case "reasoning":
			return reasoning(block.content || block.text);
		case "todo":
			return null;
		case "tool_request": {
			const result = paired ? paired.result : "";
			const items = paired && Array.isArray(paired.ok) ? paired.ok : [];
			const failed = items.some((item) => item && item.ok === false);
			return toolView(
				block.name,
				parsed(block.args),
				block.intent,
				paired ? (failed ? "failed" : "done") : "running",
				result,
				paired ? paired.duration_ms : 0,
				"",
			);
		}
		case "tool_result": {
			if (paired) return null;
			const items = Array.isArray(block.ok) ? block.ok : [];
			const failed = items.some((item) => item && item.ok === false);
			return toolView(block.name, {}, "", failed ? "failed" : "done", block.result, block.duration_ms, "");
		}
		default:
			return el("div", { class: "md", html: body(String(block.content || block.text || "")) });
	}
}

// ─── the live pieces ─────────────────────────────────────────────────────

function liveView(item) {
	switch (item.type) {
		case "text":
			return el("div", { class: "md cursor", html: body(item.text) });
		case "thinking":
			return reasoning(item.text);
		case "break":
			return el("hr");
		case "wake":
			return wake(item.text);
		case "notice":
			return el(
				"div",
				{ class: "note", "data-level": item.level === "error" ? "error" : item.level === "warning" ? "warn" : null },
				el("span", { class: "src", text: "notice" }),
				el("span", { text: item.text }),
			);
		case "tool":
			return toolView(item.name, parsed(item.args), item.intent, item.status, item.result, item.took, item.note);
		default:
			return null;
	}
}

// ─── the transcript ──────────────────────────────────────────────────────

function who(label, extra) {
	return el(
		"div",
		{ class: "who" },
		el("span", { class: "accent", text: label }),
		(extra || []).filter(Boolean).map((part) => el("span", { text: part })),
	);
}

function transcript() {
	const sheet = clear(at("sheet"));
	const held = state.session;
	if (!held) return;
	if (!held.structure.length && !held.live.length) {
		sheet.append(el("div", { class: "empty", text: "this session is empty — say something" }));
	}
	for (const round of held.structure) {
		const turn = el("section", { class: "turn" });
		turn.append(who(round.compact ? "compaction" : "round " + round.turn));
		for (const { block, paired } of page.blocksOf(held, round)) {
			const view = blockView(block, paired);
			if (view) turn.append(view);
		}
		sheet.append(turn);
	}
	if (held.live.length) {
		const turn = el("section", { class: "turn" });
		turn.append(who("talking", held.busy ? ["· streaming"] : []));
		for (const item of held.live) {
			const view = liveView(item);
			if (view) turn.append(view);
		}
		sheet.append(turn);
	}
	if (state.refusal) {
		sheet.append(
			el(
				"div",
				{ class: "note", "data-level": "error" },
				el("span", { class: "src", text: "refused" }),
				el("span", { text: state.refusal }),
				el("button", { class: "icon", text: "×", onclick: () => page.dismiss() }),
			),
		);
	}
}

// ─── the panels ──────────────────────────────────────────────────────────

function usagePanel(box) {
	const held = state.session;
	if (!held || !held.usage) {
		box.append(el("div", { class: "empty", text: "no bill yet" }));
		return;
	}
	const bill = held.usage;
	const size = page.window();
	box.append(
		el(
			"dl",
			{},
			el("dt", { text: "prompt" }),
			el("dd", { text: tokens(bill.prompt_tokens) }),
			el("dt", { text: "completion" }),
			el("dd", { text: tokens(bill.completion_tokens) }),
			el("dt", { text: "cached" }),
			el("dd", { text: tokens(bill.cached_tokens) }),
			el("dt", { text: "hit rate" }),
			el("dd", { text: Math.round((bill.cache_hit_rate || 0) * 100) + "%" }),
			el("dt", { text: "cny" }),
			el("dd", { text: money(bill.cny) || "—" }),
			el("dt", { text: "usd" }),
			el("dd", { text: money(bill.usd, "usd") || "—" }),
			el("dt", { text: "context" }),
			el("dd", { text: (bill.context ? tokens(bill.context) : "—") + (bill.estimated ? " est." : "") }),
		),
	);
	if (bill.context && size) {
		const part = Math.min(1, bill.context / size);
		box.append(
			el("div", { class: "bar" }, el("span", { style: "width:" + Math.round(part * 100) + "%" })),
			el("div", { class: "hint", text: Math.round(part * 100) + "% of " + tokens(size) }),
		);
	}
	const sub = bill.subagent_price;
	if (sub) {
		box.append(
			el("h2", { text: "subagents" }),
			el("div", { class: "hint", text: [money(sub.cny), money(sub.usd, "usd")].filter(Boolean).join(" / ") }),
		);
	}
	if (bill.groups && bill.groups.length) {
		box.append(
			el("h2", { text: "models" }),
			el(
				"ul",
				{},
				bill.groups.map((group) =>
					el(
						"li",
						{},
						el("span", { class: "fill", title: group.address, text: group.address }),
						el("span", { class: "dim", text: tokens(group.prompt_tokens + group.completion_tokens) }),
						el("span", { class: "dim", text: Math.round((group.share || 0) * 100) + "%" }),
					),
				),
			),
		);
	}
}

function todoPanel(box) {
	const list = (state.session && state.session.todo) || [];
	if (!list.length) {
		box.append(el("div", { class: "empty", text: "no checklist" }));
		return;
	}
	const done = list.filter((item) => item && item.done).length;
	box.append(el("div", { class: "hint", text: done + " of " + list.length + " done" }));
	box.append(
		el(
			"ul",
			{},
			list.map((item) =>
				el("li", {}, el("span", { class: "fill", text: (item && item.done ? "✓ " : "· ") + ((item && item.text) || "") })),
			),
		),
	);
}

function memoPanel(box) {
	const held = state.session;
	if (held && held.read) {
		box.append(
			el(
				"div",
				{ class: "row" },
				el("button", { class: "btn", text: "back", onclick: () => page.closeMemo() }),
				el("span", { class: "hint", text: held.read.scope + " · " + held.read.name }),
			),
			el(
				"ul",
				{},
				(held.read.versions || []).map((version) =>
					el(
						"li",
						{},
						el("span", { class: "dim", text: "#" + version.id }),
						el("span", { class: "fill", text: short((version.content || "").replace(/\s+/g, " "), 60) }),
					),
				),
			),
			el("div", { class: "card" }, el("div", { class: "body", text: (held.read.versions || []).slice(-1)[0]?.content || "" })),
			el(
				"div",
				{ class: "row" },
				el("button", {
					class: "btn danger",
					text: "del",
					onclick: () => page.writeMemo("del", held.read.scope, held.read.name, ""),
				}),
			),
		);
		return;
	}
	const list = (held && held.memos) || [];
	if (!list.length) box.append(el("div", { class: "empty", text: "no memos" }));
	for (const memo of list) {
		box.append(
			el(
				"div",
				{ class: "card", onclick: () => page.readMemo(memo.scope, memo.name) },
				el("span", { class: "name", text: memo.name + " · " + memo.scope }),
				el("span", { class: "body", text: short(memo.content, 160) }),
			),
		);
	}
	const name = el("input", { placeholder: "name", spellcheck: "false" });
	const content = el("textarea", { placeholder: "content", spellcheck: "false" });
	const scope = el(
		"select",
		{},
		el("option", { value: "global", text: "global" }),
		el("option", { value: "current", text: "current" }),
	);
	box.append(
		el("h2", { text: "write" }),
		el("div", { class: "row" }, scope, name),
		el("div", { class: "row" }, content),
		el(
			"div",
			{ class: "row" },
			el("button", {
				class: "btn primary",
				text: "add",
				onclick: () => page.writeMemo("add", scope.value, name.value.trim(), content.value),
			}),
			el("button", {
				class: "btn",
				text: "mod",
				onclick: () => page.writeMemo("mod", scope.value, name.value.trim(), content.value),
			}),
			el("button", {
				class: "btn",
				text: "del",
				onclick: () => page.writeMemo("del", scope.value, name.value.trim(), ""),
			}),
		),
	);
}

function treePanel(box) {
	const rounds = (state.session && state.session.structure) || [];
	if (!rounds.length) {
		box.append(el("div", { class: "empty", text: "no rounds" }));
		return;
	}
	const list = el("div", {});
	for (const round of rounds) {
		const ids = (round.ranges || []).map(([lo, hi]) => lo + "–" + hi).join(" ");
		list.append(
			el(
				"div",
				{ class: "pick", title: "cut the history back to this round's last block" },
				el("span", { class: "id", text: "#" + round.turn }),
				el("span", { class: "fill", text: (round.compact ? "compaction · " : "") + ids }),
				el("button", {
					class: "btn danger",
					text: "rewind",
					onclick: (event) => {
						event.stopPropagation();
						const last = (round.ranges || []).slice(-1)[0];
						if (last) page.rewind(round.turn, last[0]);
					},
				}),
			),
		);
	}
	box.append(list);
}

function childPanel(box) {
	const held = state.session;
	const rows = (held && held.children) || [];
	box.append(el("div", { class: "row" }, el("button", { class: "btn", text: "refresh", onclick: () => page.kids() })));
	if (!rows.length) box.append(el("div", { class: "empty", text: "no subagents" }));
	for (const row of rows) {
		box.append(
			el(
				"div",
				{ class: "pick", onclick: () => page.enter(row.id) },
				el("span", { class: "id", text: "#" + row.id }),
				el("span", { class: "fill", text: short(row.name || row.first_user || "", 40) }),
				el("span", { class: "dim", text: row.running ? "running" : row.rounds + "r" }),
			),
		);
	}
}

function setupPanel(box) {
	const held = state.session;
	if (!held) return;
	const meta = held.meta || {};
	const name = el("input", { placeholder: "name", value: meta.name || "", spellcheck: "false" });
	const path = el("input", { placeholder: "directory", value: meta.cwd || "", spellcheck: "false" });
	box.append(
		el(
			"dl",
			{},
			el("dt", { text: "provider" }),
			el("dd", { text: meta.provider || "default" }),
			el("dt", { text: "model" }),
			el("dd", { text: meta.model || "default" }),
			el("dt", { text: "effort" }),
			el("dd", { text: held.effort || "auto" }),
			el("dt", { text: "blocks" }),
			el("dd", { text: String(held.blocks.size) }),
		),
		el("h2", { text: "name" }),
		el("div", { class: "row" }, name, el("button", { class: "btn", text: "set", onclick: () => page.rename(name.value) })),
		el("h2", { text: "directory" }),
		el("div", { class: "row" }, path, el("button", { class: "btn", text: "move", onclick: () => page.move(path.value) })),
	);
	const models = (state.pools && state.pools.models) || [];
	if (models.length) {
		const pick = el(
			"select",
			{},
			models.map((model) => el("option", { value: model.provider + ":" + model.id, text: model.address })),
		);
		box.append(
			el("h2", { text: "switch" }),
			el("div", { class: "row" }, pick),
			el(
				"div",
				{ class: "row" },
				el("button", {
					class: "btn",
					text: "this session",
					onclick: () => {
						const [provider, id] = pick.value.split(":");
						page.switchModel(provider, id);
					},
				}),
				el("button", {
					class: "btn",
					text: "default",
					onclick: () => {
						const [provider, id] = pick.value.split(":");
						page.defaultModel(provider, id);
					},
				}),
			),
		);
	}
	const rungs = ["auto", "low", "medium", "high"];
	box.append(
		el("h2", { text: "effort" }),
		el("div", { class: "row" }, rungs.map((rung) => el("button", { class: "btn", text: rung, onclick: () => page.setEffort(rung) }))),
	);
	const focus = el("input", { placeholder: "focus (optional)", spellcheck: "false" });
	box.append(
		el("h2", { text: "compact" }),
		el("div", { class: "row" }, focus, el("button", { class: "btn danger", text: "now", onclick: () => page.compact(focus.value) })),
	);
}

const PANELS = [
	["usage", usagePanel],
	["todo", todoPanel],
	["memo", memoPanel],
	["tree", treePanel],
	["child", childPanel],
	["setup", setupPanel],
];

function panels() {
	document.body.dataset.panel = state.panel.open ? "open" : "closed";
	const nav = clear(at("tabs"));
	for (const [name] of PANELS) {
		nav.append(
			el("button", {
				"data-on": state.panel.tab === name ? "1" : null,
				text: name,
				onclick: () => page.tab(name),
			}),
		);
	}
	const box = clear(at("panel-body"));
	if (!state.session) {
		box.append(el("div", { class: "empty", text: "pick a session first" }));
		return;
	}
	const found = PANELS.find(([name]) => name === state.panel.tab) || PANELS[0];
	found[1](box);
}

// ─── boot ────────────────────────────────────────────────────────────────

export function draw() {
	rail();
	topbar();
	transcript();
	panels();
	const held = state.session;
	at("stop").hidden = !(held && held.busy);
	at("queue").textContent = held && held.queued.length ? "queued · " + held.queued.join(" · ") : "";
}

export { el, clear, at };