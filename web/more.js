// The top link row stays on one line: what doesn't fit moves into "⋯" (More).
// Without JS the row simply wraps.

export function initMore() {
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

  const close = (focus) => {
    if (!more.open) return;
    more.open = false;
    if (focus) more.querySelector("summary")?.focus();
  };
  more.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
    }
  });
  document.addEventListener("click", (e) => {
    if (!more.contains(e.target)) close(false);
  });
  more.addEventListener("focusout", (e) => {
    if (e.relatedTarget && !more.contains(e.relatedTarget)) close(false);
  });
}
