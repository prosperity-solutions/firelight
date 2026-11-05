// Live template islands: fetch GET <form action>?<fields>&format=json on input
// (debounced 200 ms) and render the links with DOM APIs only.
import { debounce, getJson, h, icon, safeUrl } from "./util.js";

function linkItem(l) {
  const url = safeUrl(l.url);
  if (!url) return null;
  const a = h("a", { className: "link", href: url }, icon(l.icon), h("span", { className: "label", textContent: String(l.label ?? url) }));
  return h("li", {}, a);
}

function initIsland(island) {
  const form = island.querySelector("form.template-form");
  const list = island.querySelector(".template-links");
  const error = island.querySelector(".template-error");
  if (!form || !list) return;
  let seq = 0;

  const fail = (msg) => {
    list.replaceChildren();
    list.dataset.filled = "false";
    if (error) {
      error.textContent = msg;
      error.hidden = !msg;
    }
  };

  async function run() {
    const n = ++seq;
    const params = new URLSearchParams();
    for (const input of form.querySelectorAll("input[name]")) {
      if (!input.value.trim()) return fail("");
      params.set(input.name, input.value.trim());
    }
    const url = new URL(form.getAttribute("action") || location.pathname, location.href);
    for (const [k, v] of params) url.searchParams.set(k, v);
    url.searchParams.set("format", "json");
    try {
      const data = await getJson(url.pathname + url.search);
      if (n !== seq) return; // a newer request superseded this one
      if (!data || !data.ok) return fail(String(data && data.error ? data.error : "Something went wrong."));
      const items = (Array.isArray(data.links) ? data.links : []).map(linkItem).filter(Boolean);
      list.replaceChildren(...items);
      list.dataset.filled = "true";
      if (error) error.hidden = true;
    } catch {
      if (n === seq) fail("Could not load the links.");
    }
  }

  const later = debounce(run, 200);
  form.addEventListener("input", later);
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    run();
  });

  // "#<island ref>:pr=1234" pre-fills the form and fills it on load.
  let hash = "";
  try {
    hash = decodeURIComponent(location.hash.slice(1));
  } catch {
    /* malformed hash: ignore */
  }
  const ref = island.dataset.ref || "";
  if (ref && hash.startsWith(ref + ":")) {
    for (const [k, v] of new URLSearchParams(hash.slice(ref.length + 1))) {
      const named = [...form.querySelectorAll("input[name]")].find((i) => i.name === k);
      if (named) named.value = v;
    }
    run();
  }
}

export function initTemplates() {
  for (const island of document.querySelectorAll(".island.template")) initIsland(island);
}
