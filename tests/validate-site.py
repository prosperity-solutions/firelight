#!/usr/bin/env python3
"""Validate the Firelight website (HANDOVER section 15.1). Standard library only.

Modelled on prosperity-solutions/outfit tests/validate-site.py (MIT).
Run from anywhere: python3 tests/validate-site.py
"""
import html
import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
CANONICAL = "https://firelight.oss.life.li/"
INSTALL_LINE = "curl -fsSL https://firelight.oss.life.li/install.sh | sh"
PAGES = ["index.html", "404.html"]
FOOTER_LINKS = [
    "https://github.com/prosperity-solutions/firelight",
    "https://github.com/prosperity-solutions/firelight/tree/main/docs",
    "https://firelight.oss.life.li/llms.txt",
    "https://github.com/prosperity-solutions/firelight/blob/main/LICENSE",
    "https://liechtensteinlife.com/en-DE/markets/de/legal-notice-ps",
    "https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement",
]
FOOTER_TEXT = [
    "This site sets no cookies and runs no analytics. It is hosted on",
    "Firelight · by Prosperity Solutions · built with Claude Code",
]

errors = []
notes = []


def fail(msg):
    errors.append(msg)


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.h1 = 0
        self.title = ""
        self.in_title = False
        self.metas = []
        self.links = []  # (tag, attrs)
        self.hrefs = []
        self.srcs = []
        self.inline_style = 0
        self.inline_script = 0
        self.handlers = []
        self.style_attrs = 0
        self.pre = {}
        self._pre_id = None
        self.text = ""
        self.in_script = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "h1":
            self.h1 += 1
        elif tag == "title":
            self.in_title = True
        elif tag == "meta":
            self.metas.append(a)
        elif tag == "link":
            self.links.append(a)
            if a.get("href") and a.get("rel") != "canonical":
                self.srcs.append(a["href"])
        elif tag == "a" and a.get("href"):
            self.hrefs.append(a["href"])
        elif tag == "style":
            self.inline_style += 1
        elif tag == "script":
            if a.get("src"):
                self.srcs.append(a["src"])
            else:
                self.inline_script += 1
        elif tag in ("img", "source", "iframe", "audio", "video", "embed"):
            if a.get("src"):
                self.srcs.append(a["src"])
        elif tag == "form":
            self.srcs.append(a.get("action", ""))
        if tag == "pre":
            self._pre_id = a.get("id")
            if self._pre_id:
                self.pre[self._pre_id] = ""
        for k in a:
            if k.startswith("on"):
                self.handlers.append(f"{tag}[{k}]")
            if k == "style":
                self.style_attrs += 1

    def handle_endtag(self, tag):
        if tag == "title":
            self.in_title = False
        if tag == "pre":
            self._pre_id = None

    def handle_data(self, data):
        if self.in_title:
            self.title += data
        if self._pre_id:
            self.pre[self._pre_id] += data
        self.text += data


def is_remote(u):
    return bool(urlparse(u).netloc) or u.startswith("//")


def check_page(name):
    path = SITE / name
    if not path.exists():
        fail(f"{name}: missing")
        return None
    raw = path.read_text(encoding="utf-8")
    p = Page()
    p.feed(raw)

    if p.h1 != 1:
        fail(f"{name}: expected exactly one h1, found {p.h1}")
    title = p.title.strip()
    if not title or len(title) > 60:
        fail(f"{name}: title must be 1-60 chars, is {len(title)}")

    desc = [m.get("content", "") for m in p.metas if m.get("name") == "description"]
    if len(desc) != 1 or not 50 <= len(desc[0]) <= 160:
        fail(f"{name}: need one meta description of 50-160 chars (got {[len(d) for d in desc]})")

    csp = [m.get("content", "") for m in p.metas if (m.get("http-equiv") or "").lower() == "content-security-policy"]
    if len(csp) != 1 or not csp[0].startswith("default-src 'none'"):
        fail(f"{name}: CSP meta tag must exist and start with default-src 'none'")
    elif "'unsafe-inline'" in csp[0] or "http" in csp[0]:
        fail(f"{name}: CSP must not allow inline or remote sources")

    canon = [l.get("href") for l in p.links if l.get("rel") == "canonical"]
    ogurl = [m.get("content") for m in p.metas if m.get("property") == "og:url"]
    if name == "index.html" or canon:
        if canon != [CANONICAL]:
            fail(f"{name}: canonical must be {CANONICAL}, got {canon}")
        if ogurl != canon:
            fail(f"{name}: og:url {ogurl} != canonical {canon}")

    if not any("icon" in (l.get("rel") or "") for l in p.links):
        fail(f"{name}: missing favicon link")

    if p.inline_style or p.style_attrs:
        fail(f"{name}: inline CSS found")
    if p.inline_script:
        fail(f"{name}: inline script found")
    if p.handlers:
        fail(f"{name}: event handler attributes: {p.handlers}")

    for ref in p.srcs:
        if is_remote(ref):
            fail(f"{name}: loads from another host: {ref}")
            continue
        if ref.startswith(("#", "data:", "mailto:")) or not ref:
            if ref.startswith("data:") or not ref:
                fail(f"{name}: unexpected reference {ref!r}")
            continue
        target = (SITE / ref.split("#")[0].split("?")[0].lstrip("/"))
        if not target.exists():
            fail(f"{name}: local reference does not exist: {ref}")
    for ref in p.hrefs:
        if is_remote(ref) or ref.startswith(("#", "mailto:")):
            continue
        rel = ref.split("#")[0].split("?")[0].lstrip("/")
        if rel and not (SITE / rel).exists():
            fail(f"{name}: local link does not exist: {ref}")

    for link in FOOTER_LINKS:
        if link not in p.hrefs:
            fail(f"{name}: footer link missing: {link}")
    for t in FOOTER_TEXT:
        if t not in p.text:
            fail(f"{name}: footer text missing: {t!r}")
    for label in ["GitHub", "Docs", "llms.txt", "MIT License", "Legal notice"]:
        if label not in p.text:
            fail(f"{name}: footer label missing: {label}")
    return p


def luminance(hexcol):
    h = hexcol.lstrip("#")
    if len(h) == 3:
        h = "".join(c * 2 for c in h)
    chans = []
    for i in (0, 2, 4):
        c = int(h[i:i + 2], 16) / 255
        chans.append(c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4)
    return 0.2126 * chans[0] + 0.7152 * chans[1] + 0.0722 * chans[2]


def ratio(a, b):
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def check_css():
    css_path = SITE / "style.css"
    if not css_path.exists():
        fail("style.css: missing")
        return []
    css = css_path.read_text(encoding="utf-8")
    if "@import" in css:
        fail("style.css: @import found")
    for m in re.finditer(r"url\(\s*['\"]?([^)'\"]+)", css):
        if is_remote(m.group(1)) or m.group(1).startswith("data:"):
            fail(f"style.css: remote or data url(): {m.group(1)}")
    if "@font-face" in css:
        fail("style.css: web fonts are not allowed")
    if "gradient" in css:
        fail("style.css: gradients are not allowed")

    def vars_of(block):
        return dict(re.findall(r"--([\w-]+)\s*:\s*(#[0-9a-fA-F]{3,6})\s*;", block))

    root = re.search(r":root\s*\{(.*?)\}", css, re.S)
    dark = re.search(r"prefers-color-scheme:\s*dark\)\s*\{\s*:root\s*\{(.*?)\}", css, re.S)
    if not root or not dark:
        fail("style.css: need :root and a prefers-color-scheme: dark :root block")
        return []
    light = vars_of(root.group(1))
    night = {**light, **vars_of(dark.group(1))}
    out = []
    for mode, v in (("light", light), ("dark", night)):
        for fg in ("ink", "ember", "muted"):
            for bg in ("paper", "code-bg"):
                if fg not in v or bg not in v:
                    fail(f"style.css: {mode} mode lacks --{fg} or --{bg}")
                    continue
                r = ratio(v[fg], v[bg])
                out.append(f"{mode} {fg}/{bg} {r:.1f}")
                if r < 4.5:
                    fail(f"style.css: {mode} contrast --{fg} on --{bg} is {r:.2f} (< 4.5)")
    return out


def main():
    pages = {n: check_page(n) for n in PAGES}
    contrast = check_css()

    index = pages.get("index.html")
    if index is not None:
        cmd = index.pre.get("cmd", "").strip()
        if cmd != INSTALL_LINE:
            fail(f"index.html: pre#cmd is {cmd!r}, expected {INSTALL_LINE!r}")
        if not index.pre.get("prompt", "").strip():
            fail("index.html: pre#prompt missing or empty")

    # The install line must be identical everywhere it appears (one message).
    for rel in ("README.md", "docs/install.md", "site/llms.txt"):
        f = ROOT / rel
        if not f.exists() or INSTALL_LINE not in f.read_text(encoding="utf-8"):
            fail(f"{rel}: must contain the install line {INSTALL_LINE!r}")

    # The onboarding prompt on the page is the one in the binary.
    src = (ROOT / "crates/firelight-server/src/onboarding.rs").read_text(encoding="utf-8")
    m = re.search(r'pub const PROMPT: &str = "(.*?)";\n', src, re.S)
    if not m:
        fail("onboarding.rs: PROMPT not found")
    else:
        want = m.group(1).replace('\\"', '"').replace("\\\\", "\\")
        if html.unescape(index.pre.get("prompt", "")).strip() != want.strip():
            fail("index.html: pre#prompt differs from PROMPT in crates/firelight-server/src/onboarding.rs")

    sitemap = SITE / "sitemap.xml"
    if not sitemap.exists() or f"<loc>{CANONICAL}</loc>" not in sitemap.read_text(encoding="utf-8"):
        fail("sitemap.xml: must list the canonical URL")
    robots = SITE / "robots.txt"
    rtxt = robots.read_text(encoding="utf-8") if robots.exists() else ""
    if "Sitemap: https://firelight.oss.life.li/sitemap.xml" not in rtxt:
        fail("robots.txt: Sitemap line missing")
    if "Allow: /" not in rtxt:
        fail("robots.txt: Allow: / missing")
    cname = SITE / "CNAME"
    if not cname.exists() or cname.read_text(encoding="utf-8").strip() != "firelight.oss.life.li":
        fail("CNAME: must contain firelight.oss.life.li")
    for f in ("favicon.svg", "screenshot.png", "site.js"):
        if not (SITE / f).exists():
            fail(f"{f}: missing")
    for f in ("favicon.svg",):
        p = SITE / f
        if p.exists() and re.search(r"(href|src)\s*=\s*['\"]https?:|url\(\s*['\"]?https?:", p.read_text(encoding="utf-8")):
            fail(f"{f}: external reference")

    if errors:
        print("site FAILED:")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"site OK: {len(PAGES)} pages; contrast {', '.join(contrast)}")
    for n in notes:
        print(f"note: {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
