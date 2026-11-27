// The numbers and the one-line cuts the page prints: what a bill says, how long a call took, and how much of a word
// a row has room for. Nothing here knows a region or builds a node.

export function short(text, limit) {
  const said = String(text ?? "")
    .replace(/\s+/g, " ")
    .trim();
  if (said.length <= limit) return said;
  return said.slice(0, limit - 1) + "…";
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

// A tool result said as one word: its text, whatever shape the row put it in.
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

// The one line a folded tool card shows: what the call is doing, or what it was asked.
export function note(name, args, intent) {
  if (intent) return intent;
  const said = args || {};
  const has = (key) => (typeof said[key] === "string" ? said[key] : "");
  switch (name) {
    case "read":
    case "write":
    case "edit":
    case "multiedit":
      return short(has("path"), 60);
    case "bash":
      return short(has("command"), 60);
    case "grep":
    case "rglob":
    case "search":
    case "find":
      return short(has("pattern") || has("glob") || has("paths"), 60);
    case "web_search":
      return short(has("query"), 60);
    case "web_read":
    case "web_fetch":
      return short(has("url"), 60);
    case "task":
      return short(has("agent") || has("description"), 60);
    case "todo":
      return short(has("subject") || has("text"), 60);
    case "memo":
      return short([has("op"), has("name")].filter(Boolean).join(" "), 60);
    case "kairos":
      return short(has("content") || has("intent"), 60);
    default: {
      for (const key of ["path", "command", "query", "name", "url", "text", "content"]) {
        if (has(key)) return short(has(key), 60);
      }
      return "";
    }
  }
}
