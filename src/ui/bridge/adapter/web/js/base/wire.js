// The door: one question with its answer, and the one connection a conversation streams down.

export class Refused extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

// The name the page answers to; the bridge reads it off the head and nothing else.
const ADAPTER = "webui";

// The server refuses with a keyed text; the door refuses with a code. Both read the same here.
function reason(bad) {
  if (!bad) return "refused";
  if (bad.text) return bad.text;
  if (bad.key) {
    const args = bad.args && Object.keys(bad.args).length
      ? " " + JSON.stringify(bad.args)
      : "";
    return bad.key + args;
  }
  return bad.message || bad.code || "refused";
}

// One frame, one answer: the payload the server wrote, or the refusal as it came.
async function ask(kind, name, payload = {}) {
  let res;
  try {
    res = await fetch("/", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "X-Bridge-Adapter": ADAPTER,
      },
      body: JSON.stringify({ kind, name, payload }),
    });
  } catch (e) {
    throw new Refused(
      "unreachable",
      "the bridge is not answering: " + e.message,
    );
  }
  const body = await res.json().catch(() => null);
  if (body && body.error) {
    throw new Refused(body.error.code || "refused", reason(body.error));
  }
  if (!res.ok || !body) {
    throw new Refused(
      "http_" + res.status,
      "HTTP " + res.status + " on " + kind + "/" + name,
    );
  }
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

// A question only this machine's door answers: the names one directory holds.
export function host(name, payload) {
  return ask("host", name, payload);
}

const LATER = [500, 1000, 2000, 4000, 8000];

// One conversation's frames. The bridge replays the round in flight, so a reconnect loses nothing.
export function follow(session, onFrame, onGone) {
  let closed = false;
  let tries = 0;
  const stop = () => {
    closed = true;
  };

  const run = async () => {
    while (!closed) {
      try {
        const res = await fetch("/", {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "X-Bridge-Adapter": ADAPTER,
          },
          body: JSON.stringify({
            kind: "subscribe",
            name: "watch",
            payload: { session },
          }),
        });
        if (!res.ok || !res.body) throw new Error("HTTP " + res.status);
        tries = 0;
        const reader = res.body.pipeThrough(new TextDecoderStream())
          .getReader();
        let held = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          held += value;
          let cut;
          while ((cut = held.indexOf("\n\n")) >= 0) {
            const block = held.slice(0, cut);
            held = held.slice(cut + 2);
            for (const line of block.split("\n")) {
              if (!line.startsWith("data: ")) continue;
              try {
                onFrame(JSON.parse(line.slice(6)));
              } catch {
                // A frame the door never wrote is not a frame.
              }
            }
          }
        }
      } catch (e) {
        if (closed) return;
        onGone && onGone(e);
      }
      if (closed) return;
      const wait = LATER[Math.min(tries, LATER.length - 1)];
      tries += 1;
      await new Promise((done) => setTimeout(done, wait));
    }
  };

  run();
  return { close: stop };
}
