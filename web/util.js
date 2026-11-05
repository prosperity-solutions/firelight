// Small shared helpers: safe storage, DOM builders, debounce, JSON fetch.

export const root = document.documentElement;

/** Read a localStorage value; null when storage is unavailable. */
export function load(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Write a localStorage value; silently ignored when storage is unavailable. */
export function save(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode or blocked storage: the page still works */
  }
}

/** The current space ("platform", "all", ...) or "" on pages without one. */
export function space() {
  return root.dataset.space || "";
}

/** Create an element with properties (className, textContent, ...) and children. */
export function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "dataset") Object.assign(node.dataset, v);
    else if (k in node) node[k] = v;
    else node.setAttribute(k, v);
  }
  node.append(...children);
  return node;
}

/** <svg class="icon"><use href=".../icons.svg#name"></use></svg>, built with DOM APIs. */
export function icon(name) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("class", "icon");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(ns, "use");
  const safe = String(name || "link").replace(/[^a-z0-9-]/gi, "");
  use.setAttribute("href", new URL("icons.svg", import.meta.url).href + "#" + safe);
  svg.append(use);
  return svg;
}

/** Only http(s) and same-site relative URLs may become hrefs. */
export function safeUrl(url) {
  try {
    const u = new URL(url, location.href);
    // Check the parsed scheme, but keep the server's string as written.
    return (u.protocol === "http:" || u.protocol === "https:") && /^https?:\/\//i.test(String(url)) ? String(url) : "";
  } catch {
    return "";
  }
}

export function debounce(fn, ms) {
  let t = 0;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

/** GET a JSON document from our own origin. */
export async function getJson(url) {
  const res = await fetch(url, { headers: { accept: "application/json" }, credentials: "same-origin" });
  if (!res.ok && res.status >= 500) throw new Error("HTTP " + res.status);
  return res.json();
}

/** Same-page elements by data-ref (no selector escaping needed). */
export function byRef(ref, selector = "[data-ref]") {
  return [...document.querySelectorAll(selector)].filter((e) => e.dataset.ref === ref);
}

/** Open a dialog, closing it again on a click on its backdrop. */
export function openDialog(dialog) {
  if (dialog && !dialog.open && typeof dialog.showModal === "function") dialog.showModal();
}

export function backdropClose(dialog) {
  dialog.addEventListener("click", (e) => {
    if (e.target !== dialog) return;
    const r = dialog.getBoundingClientRect();
    const inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
    if (!inside) dialog.close();
  });
}
