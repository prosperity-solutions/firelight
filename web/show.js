// "Show me": /s/<space>#<link ref> scrolls to the link and makes it glow for 2 s.
import { apply, reveal } from "./hide.js";
import { byRef } from "./util.js";

let timer = 0;

export function showRef(ref) {
  const el = byRef(ref, "a.link[data-ref]")[0];
  if (!el) return false;
  reveal(el);
  const menu = el.closest("details");
  if (menu) menu.open = true; // a top link that moved into "More"
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  el.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
  el.classList.remove("glow");
  void el.offsetWidth; // restart the animation when shown twice in a row
  el.classList.add("glow");
  clearTimeout(timer);
  timer = setTimeout(() => {
    el.classList.remove("glow");
    apply(); // hide it again if it was hidden
  }, 2000);
  return true;
}

function fromHash() {
  let ref = "";
  try {
    ref = decodeURIComponent(location.hash.slice(1));
  } catch {
    return;
  }
  if (ref && !ref.includes(":")) showRef(ref); // "ref:pr=1" is a template pre-fill
}

export function initShow() {
  fromHash();
  addEventListener("hashchange", fromHash);
}
