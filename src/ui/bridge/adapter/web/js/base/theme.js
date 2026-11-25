// The light: the one stored choice, applied to the page's root and to the two buttons that say it.

// The same key the head's first line reads before the sheets are applied, so the page never flashes.
const KEY = "mypi.theme";

const root = document.documentElement;

function stored() {
  try {
    return localStorage.getItem(KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

function apply(theme) {
  const light = theme === "light";
  root.dataset.theme = theme;
  root.style.colorScheme = light ? "light" : "dark";
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = light ? "#fbfbfc" : "#151517";
  const button = document.getElementById("theme");
  if (button) {
    const said = light ? "switch to the dark" : "switch to the light";
    button.title = said;
    button.setAttribute("aria-label", said);
  }
}

export function boot() {
  apply(stored());
}

export function wire() {
  const button = document.getElementById("theme");
  if (!button) return;
  button.addEventListener("click", () => {
    const next = root.dataset.theme === "light" ? "dark" : "light";
    try {
      localStorage.setItem(KEY, next);
    } catch {
      // A browser that keeps no storage still switches, it just forgets on the next load.
    }
    apply(next);
  });
}
