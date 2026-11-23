// Text as rows: a message body to HTML, and the one line a tool card shows when collapsed.

export function esc(text) {
	return String(text ?? "")
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;");
}

// One line of prose: code spans first, so the marks inside them stay literal.
export function inline(source) {
	const spans = [];
	let out = esc(source).replace(/`([^`\n]+)`/g, (_, code) => {
		spans.push(code);
		return "\u0000" + (spans.length - 1) + "\u0000";
	});
	out = out
		.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
		.replace(/(^|[^*\w])\*([^*\n]+)\*/g, "$1<em>$2</em>")
		.replace(/\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
	return out.replace(/\u0000(\d+)\u0000/g, (_, at) => "<code>" + spans[Number(at)] + "</code>");
}

const FENCE = /^\s*(```|~~~)\s*(\S*)\s*$/;
const HEADING = /^(#{1,3})\s+(.*)$/;
const QUOTE = /^>\s?(.*)$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const NUMBER = /^(\s*)\d+[.)]\s+(.*)$/;
const RULE = /^\s*([-*_])\1{2,}\s*$/;
const SEPARATOR = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)+\|?\s*$/;

function cells(line) {
	return line
		.trim()
		.replace(/^\|/, "")
		.replace(/\|$/, "")
		.split("|")
		.map((cell) => cell.trim());
}

// A whole body: fences, headings, lists, quotes, tables, and paragraphs in between.
export function body(source) {
	const lines = String(source ?? "").replace(/\r\n?/g, "\n").split("\n");
	const out = [];
	let para = [];
	let list = null;
	let quote = [];
	let index = 0;

	const flushPara = () => {
		if (para.length) out.push("<p>" + inline(para.join("\n")).replace(/\n/g, "<br>") + "</p>");
		para = [];
	};
	const flushList = () => {
		if (list) out.push("</" + list.tag + ">");
		list = null;
	};
	const flushQuote = () => {
		if (quote.length) out.push("<blockquote>" + inline(quote.join("\n")).replace(/\n/g, "<br>") + "</blockquote>");
		quote = [];
	};
	const flushAll = () => {
		flushPara();
		flushList();
		flushQuote();
	};

	while (index < lines.length) {
		const line = lines[index];
		const fence = FENCE.exec(line);
		if (fence) {
			flushAll();
			index += 1;
			const said = [];
			while (index < lines.length && !FENCE.test(lines[index])) {
				said.push(lines[index]);
				index += 1;
			}
			index += 1;
			const lang = fence[2] ? ' data-lang="' + esc(fence[2]) + '"' : "";
			out.push("<pre" + lang + "><code>" + esc(said.join("\n")) + "</code></pre>");
			continue;
		}
		if (!line.trim()) {
			flushAll();
			index += 1;
			continue;
		}
		if (RULE.test(line)) {
			flushAll();
			out.push("<hr>");
			index += 1;
			continue;
		}
		const heading = HEADING.exec(line);
		if (heading) {
			flushAll();
			const level = heading[1].length;
			out.push("<h" + level + ">" + inline(heading[2]) + "</h" + level + ">");
			index += 1;
			continue;
		}
		if (line.trim().startsWith("|") && index + 1 < lines.length && SEPARATOR.test(lines[index + 1])) {
			flushAll();
			const head = cells(line);
			index += 2;
			const rows = [];
			while (index < lines.length && lines[index].trim().startsWith("|")) {
				rows.push(cells(lines[index]));
				index += 1;
			}
			out.push(
				"<table><thead><tr>" +
					head.map((cell) => "<th>" + inline(cell) + "</th>").join("") +
					"</tr></thead><tbody>" +
					rows
						.map(
							(row) =>
								"<tr>" +
								row.map((cell) => "<td>" + inline(cell) + "</td>").join("") +
								"</tr>",
						)
						.join("") +
					"</tbody></table>",
			);
			continue;
		}
		const bullet = BULLET.exec(line);
		const number = NUMBER.exec(line);
		if (bullet || number) {
			flushPara();
			flushQuote();
			const depth = Math.min(Math.floor(((bullet || number)[1] || "").length / 2), 2);
			const want = bullet ? "ul" : "ol";
			const tag = want + (depth ? depth : "");
			if (!list || list.tag !== tag) {
				flushList();
				list = { tag };
				out.push("<" + tag + ">");
			}
			out.push("<li>" + inline((bullet || number)[2]) + "</li>");
			index += 1;
			continue;
		}
		const quoted = QUOTE.exec(line);
		if (quoted) {
			flushPara();
			quote.push(quoted[1]);
			index += 1;
			continue;
		}
		flushList();
		flushQuote();
		para.push(line);
		index += 1;
	}
	flushAll();
	return out.join("");
}

// The one line a tool card shows while collapsed: what the call is doing, or what it was asked.
export function note(name, args, intent) {
	if (intent) return intent;
	const said = args || {};
	const at = (key) => (typeof said[key] === "string" ? said[key] : "");
	switch (name) {
		case "read":
		case "write":
		case "edit":
		case "multiedit":
			return short(at("path"), 60);
		case "bash":
			return short(at("command"), 60);
		case "grep":
		case "rglob":
		case "search":
		case "find":
			return short(at("pattern") || at("glob") || at("paths"), 60);
		case "web_search":
			return short(at("query"), 60);
		case "web_read":
		case "web_fetch":
			return short(at("url"), 60);
		case "task":
			return short(at("agent") || at("description"), 60);
		case "todo":
			return short(at("subject") || at("text"), 60);
		case "memo":
			return short([at("op"), at("name")].filter(Boolean).join(" "), 60);
		case "kairos":
			return short(at("content") || at("intent"), 60);
		default: {
			for (const key of ["path", "command", "query", "name", "url", "text", "content"]) {
				if (at(key)) return short(at(key), 60);
			}
			return "";
		}
	}
}

export function short(text, limit) {
	const said = String(text ?? "").replace(/\s+/g, " ").trim();
	if (said.length <= limit) return said;
	return said.slice(0, limit - 1) + "…";
}

// A home directory is noise in a path; the tail matters more than the root.
export function shorten(path, limit = 60) {
	return short(String(path ?? "").replace(/^\/home\/[^/]+/, "~"), limit);
}

export function tokens(count) {
	const value = Number(count) || 0;
	if (value >= 1_000_000) return (value / 1_000_000).toFixed(1) + "M";
	if (value >= 1_000) return Math.round(value / 1_000) + "k";
	return String(value);
}

export function money(value, currency = "cny") {
	const amount = Number(value) || 0;
	if (!amount) return "";
	return (currency === "usd" ? "$" : "¥") + amount.toFixed(amount < 0.01 ? 4 : 2);
}

export function elapsed(ms) {
	const span = Number(ms) || 0;
	if (!span) return "";
	if (span < 1000) return span + "ms";
	if (span < 60_000) return (span / 1000).toFixed(1) + "s";
	return Math.floor(span / 60_000) + "m" + Math.round((span % 60_000) / 1000) + "s";
}

// A result the tool answered with: its text, whatever shape the row put it in.
export function resultText(result) {
	if (result == null) return "";
	if (typeof result === "string") return result;
	if (typeof result !== "object") return String(result);
	const said = result;
	if (typeof said.text === "string") return said.text;
	if (typeof said.output === "string") return said.output;
	if (typeof said.stdout === "string") return said.stdout;
	if (typeof said.summary === "string") return said.summary;
	try {
		return JSON.stringify(said, null, 2);
	} catch {
		return "";
	}
}