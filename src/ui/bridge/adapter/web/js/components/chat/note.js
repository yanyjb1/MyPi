// One line a refusal, a stream notice or a wake has to say: a template, a source, its words, and the tone the level
// gives it.

import { build, text } from "../../core/template.js";

export function noteLine(level, source, said) {
  const node = build("tpl-note");
  text(node, "src", source);
  text(node, "said", said);
  const tone = level === "error" ? "error" : level === "warning" ? "warn" : "";
  if (tone) node.dataset.level = tone;
  return node;
}
