// Info dialog (tabs), copy buttons for pre.copy, and the other dialogs' backdrops.
import { backdropClose, h, openDialog } from "./util.js";

function selectText(node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const sel = getSelection();
  if (sel) {
    sel.removeAllRanges();
    sel.addRange(range);
  }
}

/** Write text to the clipboard; resolves false when the browser refuses. */
export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

function addCopyButtons() {
  for (const pre of document.querySelectorAll("pre.copy")) {
    if (pre.querySelector(".copy-btn")) continue;
    const btn = h("button", { type: "button", className: "copy-btn", textContent: "Copy" });
    btn.addEventListener("click", async () => {
      const code = pre.querySelector("code") || pre;
      if (await copyText(code.textContent || "")) {
        btn.textContent = "Copied";
        setTimeout(() => (btn.textContent = "Copy"), 1500);
      } else {
        selectText(code);
        btn.textContent = "Press Ctrl+C";
      }
    });
    pre.prepend(btn);
  }
}

function initTabs(dialog) {
  const tabs = [...dialog.querySelectorAll('[role="tab"]')];
  const choose = (tab) => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = dialog.querySelector(`[data-panel="${t.dataset.tab}"]`);
      if (panel) panel.hidden = !on;
    }
  };
  tabs.forEach((t) => t.addEventListener("click", () => choose(t)));
  dialog.querySelector('[role="tablist"]')?.addEventListener("keydown", (e) => {
    const i = tabs.findIndex((t) => t.getAttribute("aria-selected") === "true");
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    const next = tabs[(i + step + tabs.length) % tabs.length];
    choose(next);
    next.focus();
    e.preventDefault();
  });
}

export function initInfo() {
  addCopyButtons();
  const info = document.getElementById("Info");
  if (info) {
    initTabs(info);
    backdropClose(info);
  }
  const hiddenDialog = document.getElementById("Hidden");
  if (hiddenDialog) backdropClose(hiddenDialog);
  document.addEventListener("click", (e) => {
    const b = e.target instanceof Element ? e.target.closest('[data-action="info"]') : null;
    if (b) openDialog(info);
  });
}
