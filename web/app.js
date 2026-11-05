// Firelight entry module: theme toggle, then the other features.
// Everything important works without JS; each init is independent.
import { initEntry } from "./entry.js";
import { initHide } from "./hide.js";
import { initInfo } from "./info.js";
import { initLive } from "./live.js";
import { initPalette } from "./palette.js";
import { initShow } from "./show.js";
import { initTemplates } from "./template.js";
import { load, root, save } from "./util.js";

const THEME_KEY = "firelight:v1:theme";
const MODES = ["auto", "light", "dark"];

function initTheme() {
  const stored = load(THEME_KEY);
  if (MODES.includes(stored)) root.dataset.theme = stored;
  const btn = document.querySelector('[data-action="theme"]');
  const label = () => {
    if (btn) btn.title = "Theme: " + (root.dataset.theme || "auto") + " (click to change)";
  };
  label();
  btn?.addEventListener("click", () => {
    const next = MODES[(MODES.indexOf(root.dataset.theme || "auto") + 1) % MODES.length];
    root.dataset.theme = next;
    save(THEME_KEY, next);
    label();
  });
}

for (const init of [initTheme, initHide, initInfo, initPalette, initTemplates, initEntry, initShow, initLive]) {
  try {
    init();
  } catch (err) {
    console.error(err);
  }
}
