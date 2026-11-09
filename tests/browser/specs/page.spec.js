// The page in a real browser (§13.3 "Browser"): no-JS rendering, ⌘K, Show me,
// hiding, template islands, permalinks, themes, 375 px, and axe.
const { test, expect } = require("@playwright/test");
const AxeBuilder = require("@axe-core/playwright").default;

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("the page, search and template islands work", async ({ page }) => {
    await page.goto("/s/platform");
    await expect(page.locator("section.island.cards h2", { hasText: "Environments" })).toBeVisible();
    await expect(page.locator('a.link[data-ref="acme-app/errors"]')).toHaveAttribute("href", "https://errors.acme.example");
    await page.fill("form.search input[name=q]", "admin rights");
    await page.press("form.search input[name=q]", "Enter");
    await expect(page.locator("ol.hits li a").first()).toHaveText("Request access rights");
    await page.goto("/s/platform");
    await page.fill('section.template input[name="pr"]', "4321");
    await page.click("section.template button[type=submit]");
    await expect(page.locator(".template-links a.link").first()).toHaveAttribute("href", "https://app-4321.preview.acme.example");
  });
});

test("⌘K finds a guide and opens it at its section", async ({ page }) => {
  await page.goto("/s/platform");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+k" : "Control+k");
  const input = page.locator("#Palette input");
  await expect(input).toBeFocused();
  await input.fill("admin rights");
  const first = page.locator("#Palette .results [role=option], #Palette .results li").first();
  await expect(first).toContainText("Request access rights");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/e\/acme-infra\/request-access/);
  await expect(page.locator("h1")).toHaveText("Request access rights");
});

test("Show me scrolls to the link and makes it glow", async ({ page }) => {
  await page.goto("/s/platform#acme-infra/argo-staging");
  const link = page.locator('a.link[data-ref="acme-infra/argo-staging"]').first();
  await expect(link).toHaveClass(/glow/);
  await expect(link).toBeInViewport();
});

test("Shift+Enter on a link result shows where it is", async ({ page }) => {
  await page.goto("/s/platform");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+k" : "Control+k");
  await page.locator("#Palette input").fill("argo");
  const options = page.locator("#Palette .results [role=option]");
  await expect(options.first()).toBeVisible();
  // Arrow down to the first link result.
  const kinds = await options.evaluateAll((els) => els.map((e) => e.getAttribute("aria-keyshortcuts")));
  const n = kinds.indexOf("Shift+Enter");
  expect(n).toBeGreaterThanOrEqual(0);
  for (let i = 0; i < n; i++) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Shift+Enter");
  await expect(page).toHaveURL(/\/s\/platform#acme-infra\/argo-/);
  await expect(page.locator("a.link.glow").first()).toBeInViewport();
});

test("hide a card, reload, still hidden, unhide from the dialog", async ({ page }) => {
  await page.goto("/s/platform");
  await page.click('button[data-action="edit"]');
  const card = page.locator('article.card[data-ref="acme-infra/environments/staging"]');
  await card.locator("button.eye").first().click();
  await page.click('button[data-action="edit"]');
  await expect(card).toBeHidden();
  await page.reload();
  await expect(card).toBeHidden();
  await expect(page.locator("button.hidden-count")).toBeVisible();
  await page.click("button.hidden-count");
  const dialog = page.locator("#Hidden");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: /unhide|show/i }).first().click();
  await page.keyboard.press("Escape");
  await page.reload();
  await expect(card).toBeVisible();
});

test("a template island fills live and refuses bad input", async ({ page }) => {
  await page.goto("/s/platform");
  const island = page.locator('section.template[data-ref="acme-app/previews"]');
  await island.locator('input[name="pr"]').fill("1234");
  await expect(island.locator(".template-links a.link").first()).toHaveAttribute("href", "https://app-1234.preview.acme.example");
  await island.locator('input[name="pr"]').fill("../x");
  await expect(island.locator(".template-error")).toBeVisible();
  await expect(island.locator(".template-links a.link")).toHaveCount(0);
});

test("permalinks and heading anchors", async ({ page }) => {
  await page.goto("/e/acme-infra/request-access#what-it-does-not-do");
  const h = page.locator("h2#what-it-does-not-do");
  await expect(h).toBeVisible();
  await expect(h.locator("a.anchor")).toHaveAttribute("href", "#what-it-does-not-do");
  await expect(page.locator(".prose a", { hasText: "the access model" })).toHaveAttribute("href", /\/e\/acme-infra\/access-model.*#what-it-is-not/);
});

for (const scheme of ["light", "dark"]) {
  test(`${scheme} mode has no serious accessibility findings`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    for (const url of ["/s/platform", "/e/acme-infra/request-access", "/s/all"]) {
      await page.goto(url);
      const result = await new AxeBuilder({ page }).analyze();
      const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      expect(serious.map((v) => `${url}: ${v.id} ${v.help} (${v.nodes.length})`)).toEqual([]);
    }
    // And with the search open, showing a guide and a link.
    await page.goto("/s/platform");
    await page.keyboard.press(process.platform === "darwin" ? "Meta+k" : "Control+k");
    await page.locator("#Palette input").fill("access");
    await expect(page.locator("#Palette .show-me").first()).toBeVisible();
    const open = await new AxeBuilder({ page }).include("#Palette").analyze();
    const bad = open.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
    expect(bad.map((v) => `palette: ${v.id} ${v.help} (${v.nodes.length})`)).toEqual([]);
  });
}

test("375 px wide: no horizontal scroll", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  for (const url of ["/s/platform", "/s/all", "/e/acme-infra/request-access", "/"]) {
    await page.goto(url);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, url).toBeLessThanOrEqual(0);
  }
});

test("the theme toggle sticks", async ({ page }) => {
  await page.goto("/s/platform");
  await page.click('button[data-action="theme"]');
  const t1 = await page.locator("html").getAttribute("data-theme");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", t1);
});

test("the theme button shows a different icon per state", async ({ page }) => {
  await page.goto("/s/platform");
  const btn = page.locator('button[data-action="theme"]');
  const shown = () => btn.locator("svg").evaluateAll((els) => els.filter((e) => getComputedStyle(e).display !== "none").map((e) => e.getAttribute("class")));
  const seen = [];
  for (let i = 0; i < 3; i++) {
    const s = await shown();
    expect(s).toHaveLength(1);
    seen.push(s[0]);
    await expect(btn).toHaveAttribute("aria-label", /^Theme: \w+\. Switch to \w+$/);
    await btn.click();
  }
  expect(new Set(seen).size).toBe(3);
});

test("top links that don't fit move into More, and nothing is lost", async ({ page }) => {
  await page.setViewportSize({ width: 300, height: 700 });
  await page.goto("/s/platform");
  const nav = page.locator(".bar .top-links");
  const total = await nav.locator("a.link").count();
  const more = nav.locator("details.more");
  await expect(more).toBeVisible();
  const inRow = await nav.locator("ul.row.top a.link").count();
  const inMenu = await nav.locator(".more-list a.link").count();
  expect(inRow + inMenu).toBe(total);
  expect(inMenu).toBeGreaterThan(0);
  // One line: the row never wraps.
  const rowHeight = await nav.locator("ul.row.top").evaluate((e) => e.getBoundingClientRect().height);
  expect(rowHeight).toBeLessThan(60);
  // Keyboard: open with Enter, close with Escape, focus back on the button.
  await more.locator("summary").focus();
  await page.keyboard.press("Enter");
  await expect(more.locator(".more-list a.link").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(more).not.toHaveAttribute("open", "");
  await expect(more.locator("summary")).toBeFocused();
  // Wide again: everything back in the row.
  await page.setViewportSize({ width: 1400, height: 700 });
  await expect(more).toBeHidden();
  expect(await nav.locator("ul.row.top a.link").count()).toBe(total);
});

test("the footer sits at the bottom of a short page and after a long one", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.goto("/");
  const footer = page.locator("footer.bar-bottom");
  const box = await footer.boundingBox();
  expect(Math.round(box.y + box.height)).toBe(900);
  expect(await footer.evaluate((e) => getComputedStyle(e).position)).toBe("static");
  await page.setViewportSize({ width: 1200, height: 300 });
  await page.goto("/s/platform");
  const y = await footer.evaluate((e) => e.getBoundingClientRect().top + scrollY);
  const main = await page.locator("main").evaluate((e) => e.getBoundingClientRect().bottom + scrollY);
  expect(y).toBeGreaterThanOrEqual(main - 1);
  expect((await footer.boundingBox()).y).toBeGreaterThan(300); // not pinned to the window
});

test("a link's note shows on keyboard focus of its ? and hides on Escape", async ({ page }) => {
  await page.goto("/s/platform");
  const li = page.locator('li:has(> a.link[data-ref="acme-infra/argo-staging"])').first();
  const tip = li.locator("button.tip");
  await expect(tip).toHaveAttribute("aria-label", "Argo CD: argo.staging.acme.example");
  await expect(li.locator(".tip-text")).toBeHidden();
  await tip.focus();
  await expect(li.locator(".tip-text")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(li.locator(".tip-text")).toBeHidden();
  await expect(li.locator("a.what")).toHaveAttribute("href", /\/e\/acme-infra\/argocd/);
});

test("a wide table scrolls in its own box, words stay whole", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  await page.goto("/e/acme-infra/request-access");
  const wrap = page.locator(".prose .table-wrap").first();
  const sizes = await wrap.evaluate((e) => ({ scroll: e.scrollWidth, client: e.clientWidth }));
  expect(sizes.scroll).toBeGreaterThan(sizes.client);
  const cell = await page.locator(".prose td").first().evaluate((e) => e.getBoundingClientRect().width);
  expect(cell).toBeGreaterThanOrEqual(140);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

test("repo pages: reachable from the header menu, with breadcrumbs back", async ({ page }) => {
  await page.goto("/s/platform");
  await page.locator(".space-switch summary").click();
  await page.locator('.space-list a[href="/r/acme-infra"]').click();
  await expect(page).toHaveURL(/\/r\/acme-infra$/);
  await expect(page.locator(".space-switch summary")).toHaveText("Infra");
  await expect(page.locator(".crumbs a").first()).toHaveAttribute("href", "/s/platform");
  await expect(page.locator(".entry-col h2", { hasText: "Guides" })).toBeVisible();
  await page.locator(".entry-list a", { hasText: "Request access rights" }).click();
  const crumbs = page.locator(".crumbs li");
  await expect(crumbs).toHaveText(["Platform", "Infra", "Request access rights"]);
  await expect(page.locator(".crumbs a").nth(1)).toHaveAttribute("href", "/r/acme-infra?s=platform");
});

test("eyes are line icons and flip to eye-off when hidden", async ({ page }) => {
  await page.goto("/s/platform");
  await page.click('button[data-action="edit"]');
  const eye = page.locator('li:has(> a.link[data-ref="acme-infra/argo-staging"]) .acts button.eye').first();
  await expect(eye.locator("svg.i-eye")).toBeVisible();
  await eye.click();
  await expect(eye.locator("svg.i-eye-off")).toBeVisible();
  await expect(eye).toHaveAttribute("aria-label", "Show again");
  await eye.click();
});
