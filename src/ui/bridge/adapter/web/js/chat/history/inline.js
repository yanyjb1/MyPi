import { esc } from "../../base/note.js";

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
    .replace(
      /\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g,
      '<a href="$2" target="_blank" rel="noreferrer">$1</a>',
    );
  return out.replace(
    /\u0000(\d+)\u0000/g,
    (_, at) => "<code>" + spans[Number(at)] + "</code>",
  );
}
