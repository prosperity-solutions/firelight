// The top link row stays on one line: what doesn't fit moves into "⋯" (More).
// Without JS the row simply wraps.

/** Escape and a click outside close a header menu (<details>). */
export function menuClose(menu) {
  const close = (focus) => {
    if (!menu.open) return;
    menu.open = false;
    if (focus) menu.querySelector("summary")?.focus();
  };
  menu.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
    }
  });
  document.addEventListener("click", (e) => {
    if (!menu.contains(e.target)) close(false);
  });
  menu.addEventListener("focusout", (e) => {
    if (e.relatedTarget && !menu.contains(e.relatedTarget)) close(false);
  });
}

export function initMore() {
  const sw = document.querySelector(".bar .space-switch");
  if (sw) menuClose(sw);
  const nav = document.querySelector(".bar .top-links");
  const row = nav?.querySelector("ul.row.top");
  const more = nav?.querySelector("details.more");
  const list = more?.querySelector(".more-list");
  if (!row || !more || !list) return;
  const items = [...row.children];
  nav.classList.add("fit");

  const over = () => row.scrollWidth > row.clientWidth + 1;
  function layout() {
    row.append(...items); // back in order
    more.hidden = true;
    if (!over()) return;
    more.hidden = false; // "⋯" takes its room first
    while (row.children.length && over()) list.prepend(row.lastElementChild);
  }

  let frame = 0;
  const schedule = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(layout);
  };
  layout();
  if ("ResizeObserver" in window) new ResizeObserver(schedule).observe(nav);
  else addEventListener("resize", schedule);
  // Hiding links in edit mode changes what fits.
  new MutationObserver(schedule).observe(document.body, { attributes: true, attributeFilter: ["class"] });

  menuClose(more);
}
