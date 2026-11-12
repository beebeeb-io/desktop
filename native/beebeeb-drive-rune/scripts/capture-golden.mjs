#!/usr/bin/env node
// capture-golden.mjs <frame-id> [--dark] [--out <dir>] [--board <dir>]
//
// Exports one board frame as the golden for the three gates:
//   golden/<frame-id>[-dark]-board.png   the window's client area at 2x
//   golden/<frame-id>[-dark].json        every keyed element: box, text, typography, colours, icon paths
//
// The board draws a 1 px window frame around the screen (the OS draws that one on a real
// PC), so the golden is the screen's PADDING box and every coordinate is relative to it.
// For a 1280x800 board frame that is a 1278x798 client area: render the native side at
// that size.
//
// Keys come from `keymap()` below, by the board's own class names. A frame that draws
// something new adds its keys there (see README "Add a frame").
import { createRequire } from 'module';
import { writeFileSync, mkdirSync, readFileSync, readdirSync, existsSync } from 'fs';
import { dirname, join, resolve } from 'path';
import { fileURLToPath } from 'url';
import { createServer } from 'http';

const HERE = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const flag = (name) => { const i = args.indexOf(name); return i >= 0 ? args.splice(i, 2)[1] : undefined; };
const outDir = resolve(flag('--out') || join(HERE, '..', 'golden'));
const boardDir = resolve(flag('--board') || process.env.BB_BOARD_DIR || '/home/guus/code/beebeeb.io/design/desktop-v2');
const dark = args.includes('--dark');
const frameId = args.find((a) => !a.startsWith('--'));
if (!frameId) { console.error('usage: capture-golden.mjs <frame-id> [--dark] [--out dir] [--board dir]'); process.exit(64); }

// Playwright is not a dependency of this crate: take it from PLAYWRIGHT_DIR, or the first known install.
function loadPlaywright() {
  const roots = [process.env.PLAYWRIGHT_DIR, '/home/guus/code/beebeeb.io/repos/web/node_modules',
    '/home/guus/code/beebeeb.io/node_modules', '/tmp/bunx-1000-playwright@latest/node_modules'].filter(Boolean);
  for (const root of roots) {
    for (const name of ['playwright', '@playwright/test', 'playwright-core']) {
      if (existsSync(join(root, name))) { try { return createRequire(join(root, 'x.js'))(name); } catch { /* next */ } }
    }
  }
  console.error(`capture-golden: Playwright not found (set PLAYWRIGHT_DIR to a node_modules that has it); looked in ${roots.join(', ')}`);
  process.exit(2);
}
const { chromium } = loadPlaywright();

mkdirSync(outDir, { recursive: true });
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.woff2': 'font/woff2' };
const server = createServer((req, res) => {
  const rel = decodeURIComponent(req.url.split('?')[0]);
  const path = join(boardDir, rel === '/' ? 'board.html' : rel);
  if (!path.startsWith(boardDir)) { res.writeHead(403); return res.end(); }
  try {
    const data = readFileSync(path);
    res.writeHead(200, { 'Content-Type': types[path.slice(path.lastIndexOf('.'))] || 'application/octet-stream' });
    res.end(data);
  } catch { res.writeHead(404); res.end('Not found'); }
});
await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
const port = server.address().port;

// Text as Windows and macOS draw it: fractional advances, no hinting. Linux FreeType's default
// (hinted, whole-pixel advances: JetBrains Mono 11px comes out 7px a character, not 6.6) is
// what no user of the app will ever see.
// A REAL 200 % display, not an emulated one: with Playwright's `deviceScaleFactor` the page lays out at
// 100 % and is scaled up, so every `.5px` hairline comes out a whole CSS pixel (two device pixels).
const browser = await chromium.launch({ headless: true, args: ['--force-device-scale-factor=2', '--window-size=1600,1200',
  '--font-render-hinting=none', '--enable-font-subpixel-positioning', '--disable-lcd-text'] });
let code = 0;
try {
  const page = await browser.newPage({ viewport: null });
  const dpr = await page.evaluate(() => window.devicePixelRatio);
  if (dpr !== 2) { console.error(`capture-golden: the browser runs at ${dpr}x, not 2x`); process.exit(2); }
  await page.goto(`http://127.0.0.1:${port}/board.html`, { waitUntil: 'networkidle', timeout: 60000 });
  const platform = frameId.startsWith('DM-') ? 'macOS' : 'Windows';
  for (const tab of await page.$$('.kit-switch a, .kit-tabs a')) {
    if ((await tab.textContent())?.trim() === platform) { await tab.click(); break; }
  }
  await page.waitForSelector(`[id="${frameId}"] .kit-screen`, { timeout: 15000 });
  await page.evaluate(() => document.fonts.ready);

  // Only the frame's own screen stays; sticky board chrome would be drawn over it.
  const clip = await page.evaluate(({ frameId, dark }) => {
    const screen = document.querySelector(`[id="${frameId}"] .kit-screen`);
    for (const el of document.querySelectorAll('*')) {
      const cs = getComputedStyle(el);
      if ((cs.position === 'sticky' || cs.position === 'fixed') && !screen.contains(el)) el.style.display = 'none';
    }
    if (dark) screen.classList.add('dark');
    screen.scrollIntoView({ block: 'start', inline: 'nearest', behavior: 'instant' }); // instant: a smooth scroll of the frame's row is still moving when the box is read
    window.scrollBy(0, -20);
    const r = screen.getBoundingClientRect();
    // Viewport coordinates, for a viewport screenshot: a full-page one resizes the viewport first, and a
    // frame that is not the first of its row (the board's rows scroll sideways) then moves from under the clip.
    return { x: r.x + screen.clientLeft, y: r.y + screen.clientTop, width: screen.clientWidth, height: screen.clientHeight, vw: window.innerWidth, vh: window.innerHeight };
  }, { frameId, dark });
  await page.waitForTimeout(400);

  const stem = dark ? `${frameId}-dark` : frameId;
  const png = join(outDir, `${stem}-board.png`);
  if (clip.x < 0 || clip.y < 0 || clip.x + clip.width > clip.vw || clip.y + clip.height > clip.vh) {
    console.error(`capture-golden: the frame (${clip.width}x${clip.height} at ${clip.x},${clip.y}) does not fit the ${clip.vw}x${clip.vh} viewport`); process.exit(2);
  }
  await page.screenshot({ path: png, clip: { x: clip.x, y: clip.y, width: clip.width, height: clip.height } });

  const geo = await page.evaluate(({ frameId, keymapSrc }) => {
    const screen = document.querySelector(`[id="${frameId}"] .kit-screen`);
    const sr = screen.getBoundingClientRect();
    const ox = sr.x + screen.clientLeft, oy = sr.y + screen.clientTop;
    const round = (v) => Math.round(v * 100) / 100;
    const box = (b) => ({ x: round(b.x - ox), y: round(b.y - oy), w: round(b.width), h: round(b.height) });
    const probe = document.createElement('canvas'); probe.width = probe.height = 1;
    const pctx = probe.getContext('2d', { willReadFrequently: true });
    // Computed colours come back in the notation they were written in (oklch, color-mix): resolve to sRGB bytes.
    const rgba = (css) => {
      pctx.clearRect(0, 0, 1, 1); pctx.fillStyle = '#000'; pctx.fillStyle = css; pctx.fillRect(0, 0, 1, 1);
      const d = pctx.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3]];
    };
    const typo = (el) => {
      const cs = getComputedStyle(el);
      const mono = /mono/i.test(cs.fontFamily.split(',')[0]);
      return { family: mono ? 'mono' : 'sans', fontFamily: cs.fontFamily.split(',')[0].trim().replace(/['"]/g, ''),
        size: parseFloat(cs.fontSize), weight: parseInt(cs.fontWeight, 10), color: rgba(cs.color),
        tracking: cs.letterSpacing === 'normal' ? 0 : round(parseFloat(cs.letterSpacing)) };
    };
    const rendered = (s, el) => {
      const t = getComputedStyle(el).textTransform; s = s.replace(/\s+/g, ' ').trim();
      return t === 'uppercase' ? s.toUpperCase() : t === 'lowercase' ? s.toLowerCase() : s;
    };
    const out = {}; const problems = [];
    const put = (key, rec) => { if (out[key]) problems.push(`duplicate key ${key}`); out[key] = rec; };
    const api = {
      // A box: position and size, its fill, and (px) the pixels inside it.
      box(key, el, extra = {}) {
        if (!el) return;
        const cs = getComputedStyle(el); const bg = rgba(cs.backgroundColor);
        put(key, { kind: 'box', ...box(el.getBoundingClientRect()), ...(bg[3] ? { bg } : {}), ...extra });
      },
      // An element whose whole content is one string.
      text(key, el, extra = {}) {
        if (!el) return;
        put(key, { kind: 'text', ...box(el.getBoundingClientRect()), text: rendered(el.textContent, el), ...typo(el), ...extra });
      },
      // A bare text node between other children ("New" between its two icons).
      textNode(key, parent, extra = {}) {
        if (!parent) return;
        const node = [...parent.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim());
        if (!node) { problems.push(`no text node for ${key}`); return; }
        const range = document.createRange(); range.selectNodeContents(node);
        put(key, { kind: 'text', ...box(range.getBoundingClientRect()), text: rendered(node.textContent, parent), ...typo(parent), ...extra });
      },
      // An icon: its box, its colour and WHICH glyph (the path data), filled or outlined.
      icon(key, svg, extra = {}) {
        if (!svg) return;
        const shapes = [...svg.querySelectorAll('path, rect, circle, line, polyline, polygon, ellipse')];
        const d = shapes.map((s) => s.tagName === 'path' ? s.getAttribute('d') : `${s.tagName}:${[...s.attributes].filter((a) => !/^(fill|stroke|class|style)/.test(a.name)).map((a) => `${a.name}=${a.value}`).join(',')}`);
        const first = shapes[0] && getComputedStyle(shapes[0]);
        const filled = !!first && first.fill !== 'none' && svg.getAttribute('fill') === 'none';
        put(key, { kind: 'icon', ...box(svg.getBoundingClientRect()), icon: d, filled, color: rgba(getComputedStyle(svg).color), ...extra });
      },
      problems,
    };
    // eslint-disable-next-line no-new-func
    new Function('win', 'api', keymapSrc)(screen, api);
    return { frame: frameId, width: screen.clientWidth, height: screen.clientHeight, elements: out, problems };
  }, { frameId, keymapSrc: ['keymap.js', ...readdirSync(HERE).filter((f) => /^keymap-.+\.js$/.test(f)).sort()].map((f) => readFileSync(join(HERE, f), 'utf-8')).join('\n') });

  const flat = JSON.stringify(geo);
  if (/â€|Â·/.test(flat)) { console.error('capture-golden: mojibake in the exported text (the board must be served as utf-8)'); code = 1; }
  for (const p of geo.problems) { console.error(`capture-golden: ${p}`); code = 1; }
  const count = Object.keys(geo.elements).length;
  if (count === 0) { console.error('capture-golden: the keymap matched nothing in this frame'); code = 1; }
  delete geo.problems;
  geo.theme = dark ? 'dark' : 'light';
  writeFileSync(join(outDir, `${stem}.json`), JSON.stringify(geo, null, 1) + '\n');
  console.log(`golden: ${stem}: ${count} keyed elements, client ${geo.width}x${geo.height}, ${png}`);
} finally {
  await browser.close();
  server.close();
}
process.exit(code);
