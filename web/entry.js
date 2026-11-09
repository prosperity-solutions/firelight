// Entry pages: copy a section's URL when its anchor is clicked (navigation
// still happens), and a "Show me" link next to markdown link references.
import { copyText } from "./info.js";
import { h } from "./util.js";

export function initEntry() {
  // A fresh visit to an entry or repo page starts at the top unless the URL
  // names a section; Back and Forward keep the browser's own restore.
  if (document.querySelector("main.entry") && !location.hash) {
    const nav = performance.getEntriesByType?.("navigation")?.[0];
    if (!nav || nav.type === "navigate") scrollTo(0, 0);
  }
  document.addEventListener("click", (e) => {
    const a = e.target instanceof Element ? e.target.closest(".prose a.anchor") : null;
    if (a) copyText(new URL(a.getAttribute("href") || "", location.href).href);
  });
  for (const ref of document.querySelectorAll("a.link-ref[data-show]")) {
    ref.after(h("a", { className: "show-me", href: ref.dataset.show, textContent: "Show me", title: "Show this link on the page" }));
  }
}
