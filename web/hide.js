// Edit mode and hiding. Hidden refs persist per space in
// localStorage["firelight:v1:hidden:<space>"] as a JSON array.
import { byRef, h, load, openDialog, save, space } from "./util.js";

const KEY = "firelight:v1:hidden:" + space();
let hidden = new Set();
const listeners = [];

function read() {
  try {
    const v = JSON.parse(load(KEY) || "[]");
    return new Set(Array.isArray(v) ? v.filter((x) => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function write() {
  save(KEY, JSON.stringify([...hidden]));
}

export function isHidden(ref) {
  return hidden.has(ref);
}

/** Elements that carry a ref and can be hidden: islands, cards, link items. */
function targets(ref) {
  const out = byRef(ref, ".island[data-ref], .card[data-ref]");
  for (const a of byRef(ref, "a.link[data-ref]")) {
    const li = a.closest("li");
    if (li) out.push(li);
  }
  return out;
}

function allRefs() {
  const refs = new Set();
  for (const e of document.querySelectorAll(".island[data-ref], .card[data-ref], a.link[data-ref]")) {
    refs.add(e.dataset.ref);
  }
  return refs;
}

/** Re-apply the hidden set to the page. */
export function apply() {
  for (const ref of allRefs()) {
    const on = hidden.has(ref);
    for (const el of targets(ref)) el.classList.toggle("is-hidden", on);
  }
  for (const eye of document.querySelectorAll("[data-hide]")) {
    eye.setAttribute("aria-pressed", String(hidden.has(eye.dataset.hide)));
    eye.title = hidden.has(eye.dataset.hide) ? "Show again" : "Hide";
  }
  const count = document.querySelector(".hidden-count");
  if (count) {
    const n = [...hidden].filter((r) => allRefs().has(r)).length;
    count.hidden = n === 0;
    const span = count.querySelector("span");
    if (span) span.textContent = String(n);
  }
  for (const fn of listeners) fn();
}

/** Show an element (and hidden ancestors) until the next apply(). */
export function reveal(el) {
  for (let e = el; e && e !== document.body; e = e.parentElement) e.classList.remove("is-hidden");
}

function toggle(ref) {
  if (hidden.has(ref)) hidden.delete(ref);
  else hidden.add(ref);
  write();
  apply();
}

/** Each link <li> gets an eye button once (visible only in edit mode via CSS). */
function addLinkEyes() {
  for (const a of document.querySelectorAll("a.link[data-ref]")) {
    const li = a.closest("li");
    if (!li || li.querySelector(":scope > .eye")) continue;
    li.append(
      h("button", { type: "button", className: "eye", textContent: "\u{1F441}", dataset: { hide: a.dataset.ref }, "aria-pressed": "false" }),
    );
  }
}

function label(ref) {
  const el = targets(ref)[0];
  if (!el) return ref;
  const t = el.matches("li") ? el.querySelector(".label") : el.querySelector("h2, h3");
  return (t ? t.textContent : "").trim() || ref;
}

function renderList(list) {
  list.replaceChildren();
  for (const ref of [...hidden].filter((r) => allRefs().has(r))) {
    const btn = h("button", { type: "button", textContent: "Unhide" });
    btn.addEventListener("click", () => toggle(ref));
    list.append(h("li", {}, h("span", { textContent: label(ref), title: ref }), btn));
  }
}

export function initHide() {
  hidden = read();
  const dialog = document.getElementById("hidden-dialog");
  const list = dialog && dialog.querySelector(".hidden-list");
  if (list) listeners.push(() => dialog.open && renderList(list));

  document.addEventListener("click", (e) => {
    const t = e.target instanceof Element ? e.target : null;
    if (!t) return;
    const eye = t.closest("[data-hide]");
    if (eye) return toggle(eye.dataset.hide);
    const act = t.closest("[data-action]");
    if (!act) return;
    if (act.dataset.action === "edit") {
      const on = document.body.classList.toggle("editing");
      act.setAttribute("aria-pressed", String(on));
      if (on) addLinkEyes();
      apply();
    } else if (act.dataset.action === "hidden" && dialog) {
      if (list) renderList(list);
      openDialog(dialog);
    } else if (act.dataset.action === "reset-hidden") {
      hidden = new Set();
      write();
      apply();
      if (dialog) dialog.close();
    }
  });
  apply();
}
