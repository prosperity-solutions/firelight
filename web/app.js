// Firelight entry module: theme toggle, then the other features.
// Everything important works without JS; each init is independent.
import { initEntry } from "./entry.js";
import { initHide } from "./hide.js";
import { initInfo } from "./info.js";
import { initLive } from "./live.js";
import { initMore } from "./more.js";
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
  const NAMES = { auto: "system", light: "light", dark: "dark" };
  const label = () => {
    if (!btn) return;
    const now = root.dataset.theme || "auto";
    const next = MODES[(MODES.indexOf(now) + 1) % MODES.length];
    const text = `Theme: ${NAMES[now] || now}. Switch to ${NAMES[next]}`;
    btn.title = text;
    btn.setAttribute("aria-label", text);
  };
  label();
  btn?.addEventListener("click", () => {
    const next = MODES[(MODES.indexOf(root.dataset.theme || "auto") + 1) % MODES.length];
    root.dataset.theme = next;
    save(THEME_KEY, next);
    label();
  });
}

// A "?" tooltip shows on hover and focus; Escape dismisses it (WCAG 1.4.13).
function initTips() {
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && document.activeElement?.classList.contains("tip")) document.activeElement.blur();
  });
}

// <time data-rel>: the server writes the exact time; show it as "5 minutes ago".
function initTimes() {
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const units = [["year", 31536000], ["month", 2592000], ["day", 86400], ["hour", 3600], ["minute", 60], ["second", 1]];
  for (const t of document.querySelectorAll("time[data-rel]")) {
    const ms = Date.parse(t.dateTime);
    if (Number.isNaN(ms)) continue;
    const s = Math.round((ms - Date.now()) / 1000);
    const [unit, n] = units.find(([, n]) => Math.abs(s) >= n) || units[units.length - 1];
    t.textContent = rtf.format(Math.round(s / n), unit);
  }
}

for (const init of [initTheme, initTips, initTimes, initHide, initMore, initInfo, initPalette, initTemplates, initEntry, initShow, initLive]) {
  try {
    init();
  } catch (err) {
    console.error(err);
  }
}
