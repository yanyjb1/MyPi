// The page's boot: wire the few controls, say the first frame, and redraw on every change.

import * as page from "./state.js";
import { draw } from "./view.js";

const at = (id) => document.getElementById(id);
const say = at("say");
const send = at("send");
const stop = at("stop");

function grow() {
	say.style.height = "auto";
	say.style.height = Math.min(say.scrollHeight, 176) + "px";
}

// One line: a panel's own command, or a message. The panel opens before the words leave.
function submit() {
	const text = say.value;
	if (!text.trim()) return;
	if (page.summon(text)) {
		say.value = "";
		grow();
		return;
	}
	page.send(text);
	say.value = "";
	grow();
}

say.addEventListener("input", grow);
say.addEventListener("keydown", (event) => {
	if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
		event.preventDefault();
		submit();
	}
});

send.addEventListener("click", submit);
stop.addEventListener("click", () => page.interrupt());

window.addEventListener("keydown", (event) => {
	if (event.key === "Escape" && page.state.panel.open) {
		event.preventDefault();
		page.closePanel();
	}
	if (event.key === "." && (event.ctrlKey || event.metaKey)) {
		event.preventDefault();
		page.interrupt();
	}
});

at("panel-toggle").addEventListener("click", () => {
	if (page.state.panel.open) page.closePanel();
	else page.openPanel();
});

at("panel-close").addEventListener("click", () => page.closePanel());

function fresh() {
	const where = at("where").value.trim();
	page.create(where || undefined);
}

at("new").addEventListener("click", fresh);
at("where").addEventListener("keydown", (event) => {
	if (event.key === "Enter") {
		event.preventDefault();
		fresh();
	}
});

// The rail's own clock: a session list goes stale quietly, so ask again while the tab is in front.
setInterval(() => {
	if (document.visibilityState === "visible") page.refresh();
}, 15_000);

page.watch(draw);
draw();
page.refresh().then(() => {
	if (page.state.active) page.enter(page.state.active);
	page.pools();
	say.focus();
});