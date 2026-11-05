// Live reload for `firelight preview`: poll /__live and reload when the
// content generation changes. Paused while the tab is hidden.
export function initLive() {
  const meta = document.querySelector('meta[name="firelight-live"]');
  if (!meta) return;
  const known = meta.content;
  let timer = 0;

  async function tick() {
    timer = 0;
    if (document.hidden) return;
    try {
      const res = await fetch("/__live", { cache: "no-store" });
      const data = await res.json();
      if (String(data.generation) !== known) return location.reload();
    } catch {
      /* server restarting: keep polling */
    }
    timer = setTimeout(tick, 1000);
  }

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && !timer) tick();
  });
  tick();
}
