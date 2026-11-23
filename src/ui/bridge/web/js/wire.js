// The bridge's door: one question and its answer, and the stream one conversation pushes.

export class Refused extends Error {
	constructor(code, message) {
		super(message);
		this.code = code;
	}
}

// The name the page's own adapter answers to; the bridge reads it off the query string.
const ADAPTER = "webui";

// The server refuses with a keyed text; the bridge refuses with a code. Both read the same here.
function reason(bad) {
	if (!bad) return "refused";
	if (bad.text) return bad.text;
	if (bad.key) {
		const args = bad.args && Object.keys(bad.args).length ? " " + JSON.stringify(bad.args) : "";
		return bad.key + args;
	}
	return bad.message || bad.code || "refused";
}

// One question: the answer's payload, or a refusal carrying the server's own word.
export async function ask(kind, name, payload = {}) {
	let res;
	try {
		res = await fetch("/rpc?adapter=" + ADAPTER, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({ kind, name, payload }),
		});
	} catch (e) {
		throw new Refused("unreachable", "the bridge is not answering: " + e.message);
	}
	const body = await res.json().catch(() => null);
	if (body && body.error) throw new Refused(body.error.code || "refused", reason(body.error));
	if (!res.ok || !body) throw new Refused("http_" + res.status, "HTTP " + res.status + " on " + kind + "/" + name);
	return body;
}

export function call(name, payload) {
	return ask("call", name, payload);
}

export function post(name, payload) {
	return ask("post", name, payload);
}

export function subscribe(name, payload) {
	return ask("subscribe", name, payload);
}

// One conversation's stream. The browser reconnects on its own, and the bridge replays the round in flight.
export function follow(session, onFrame, onOpen, onFail) {
	const source = new EventSource("/events?adapter=" + ADAPTER + "&session=" + encodeURIComponent(session));
	source.onopen = () => onOpen && onOpen();
	source.onmessage = (event) => {
		let frame;
		try {
			frame = JSON.parse(event.data);
		} catch {
			return;
		}
		onFrame(frame);
	};
	source.onerror = () => onFail && onFail();
	return source;
}