// The model table this page reads for context size and thinking rungs, and the one address it asks about.

import { call } from "./wire.js";
import { address, announce } from "./state.js";

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

export function entryOf(address) {
  return (address && entries.get(address)) || null;
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
  const found = entryOf(address());
  return (found && found.contextWindow) || 0;
}

export async function windowed() {
  await entry(address());
}

export function rungs() {
  const found = entryOf(address());
  const list = (found && found.reasoningEfforts) || [];
  return ["auto"].concat(list);
}
