// Command palette (Ctrl/Cmd+K, "/", or focusing the header search box).
import { isHidden } from "./hide.js";
import { showRef } from "./show.js";
import { backdropClose, debounce, getJson, h, openDialog, safeHref, space } from "./util.js";

const KINDS = { guide: "Guide", concept: "Concept", link: "Link", repo: "Repo" };

export function initPalette() {
  const dialog = document.getElementById("Palette");
  if (!dialog) return;
  const input = dialog.querySelector("input");
  const form = dialog.querySelector("form");
  const results = dialog.querySelector(".results");
  const empty = dialog.querySelector(".palette-empty");
  let hits = [];
  let index = -1;
  let seq = 0;
  let quietUntil = 0; // ignore header-search focus right after closing (focus is restored there)
  backdropClose(dialog);

  const quiet = () => {
    quietUntil = Date.now() + 500;
  };
  dialog.addEventListener("cancel", quiet);
  dialog.addEventListener("close", quiet);

  function open() {
    openDialog(dialog);
    input.focus();
    input.select();
  }

  function select(i) {
    index = hits.length ? (i + hits.length) % hits.length : -1;
    [...results.children].forEach((li, n) => {
      const on = n === index;
      li.setAttribute("aria-selected", String(on));
      if (on) {
        li.scrollIntoView({ block: "nearest" });
        input.setAttribute("aria-activedescendant", li.id);
      }
    });
  }

  function go(hit) {
    const url = safeHref(hit.url);
    if (!url) return;
    dialog.close();
    location.href = url;
  }

  function showMe(hit) {
    dialog.close();
    let target;
    const show = safeHref(hit.show);
    if (!show) return;
    try {
      target = new URL(show, location.href);
    } catch {
      return;
    }
    if (target.pathname === location.pathname) {
      const same = target.hash === location.hash;
      location.hash = target.hash;
      if (same) showRef(hit.ref);
    } else {
      location.href = target.href;
    }
  }

  function row(hit, n) {
    const li = h("li", { id: "palette-hit-" + n, role: "option", "aria-selected": "false" });
    li.append(h("span", { className: "kind", textContent: KINDS[hit.kind] || String(hit.kind || "") }));
    li.append(h("span", { className: "title", textContent: String(hit.title ?? hit.ref ?? "") }));
    if (hit.repo) li.append(h("span", { className: "repo", textContent: String(hit.repo) }));
    if (hit.kind === "link" && hit.show) {
      // Enter opens the link; this chip (and Shift+Enter) shows where it sits on
      // the page instead. A mouse shortcut, not a control, so the option holds
      // nothing interactive.
      li.setAttribute("aria-keyshortcuts", "Shift+Enter");
      li.setAttribute("aria-description", "Shift+Enter: show it on the page");
      const btn = h("span", { className: "show-me", "aria-hidden": "true", textContent: "Locate", title: "Show where it is on the page (Shift+Enter)" });
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        showMe(hit);
      });
      li.append(btn);
    }
    const line = hit.summary;
    if (line) li.append(h("span", { className: "summary", textContent: String(line), title: String(line) }));
    li.addEventListener("click", () => go(hit));
    li.addEventListener("mousemove", () => index !== n && select(n));
    return li;
  }

  function render(list, q) {
    // hidden items of this space sink to the bottom; the sort is stable
    hits = list.map((x, i) => [x, i]).sort((a, b) => Number(isHidden(a[0].ref)) - Number(isHidden(b[0].ref)) || a[1] - b[1]).map((p) => p[0]);
    results.replaceChildren(...hits.map(row));
    empty.hidden = hits.length > 0 || !q;
    index = -1;
    input.removeAttribute("aria-activedescendant");
    if (hits.length) select(0);
  }

  const search = debounce(async () => {
    const q = input.value.trim();
    const n = ++seq;
    if (!q) return render([], "");
    try {
      const data = await getJson(`/s/${encodeURIComponent(space() || "all")}/search?q=${encodeURIComponent(q)}&format=json`);
      if (n === seq) render(Array.isArray(data.hits) ? data.hits : [], q);
    } catch {
      if (n === seq) render([], q);
    }
  }, 120);

  input.addEventListener("input", search);
  form.addEventListener("submit", (e) => e.preventDefault());
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") select(index + 1);
    else if (e.key === "ArrowUp") select(index - 1);
    else if (e.key === "Enter" && e.shiftKey && hits[index]?.kind === "link" && hits[index].show) showMe(hits[index]);
    else if (e.key === "Enter" && hits[index]) go(hits[index]);
    else return;
    e.preventDefault();
  });

  const field = (t) => t instanceof Element && (t.closest("input, textarea, select, [contenteditable]") !== null);
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "k") {
      e.preventDefault();
      open();
    } else if (e.key === "/" && !e.metaKey && !e.ctrlKey && !e.altKey && !field(e.target) && !document.querySelector("dialog[open]")) {
      e.preventDefault();
      open();
    }
  });

  const header = document.querySelector(".bar .search input");
  if (header) {
    header.addEventListener("focus", () => {
      if (Date.now() < quietUntil || dialog.open) return;
      header.blur();
      open();
    });
  }
}
