// Exports the board's six sample photos (`.mk-ph1` to `.mk-ph6`) as small PNG files, in the three
// shapes the board draws them: square (grid tile, current strip thumb, dialog tile), strip (36 x 52)
// and preview (600 x 400). Usage: node export-photos.mjs <out-dir>
import { createRequire } from 'module';
import { readFileSync } from 'fs';
import { createServer } from 'http';
import { join } from 'path';
const { chromium } = createRequire('/home/guus/code/beebeeb.io/repos/web/node_modules/x.js')('@playwright/test');
const boardDir = '/home/guus/code/beebeeb.io/design/desktop-v2';
const server = createServer((req, res) => { try { const rel = decodeURIComponent(req.url.split('?')[0]); const d = readFileSync(join(boardDir, rel === '/' ? 'board.html' : rel)); res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' }); res.end(d); } catch { res.writeHead(404); res.end(); } });
await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 800, height: 600 }, deviceScaleFactor: 1 });
await page.goto(`http://127.0.0.1:${server.address().port}/board.html`, { waitUntil: 'domcontentloaded', timeout: 60000 });
const shapes = { square: [96, 96], strip: [72, 104], preview: [300, 200] };
for (let n = 1; n <= 6; n++) for (const [shape, [w, h]] of Object.entries(shapes)) {
  await page.evaluate(({ n, w, h }) => {
    document.getElementById('ph-export')?.remove();
    const el = document.createElement('span');
    el.id = 'ph-export'; el.className = `mk-ph mk-ph${n}`;
    el.style.cssText = `position:fixed;left:0;top:0;width:${w}px;height:${h}px;z-index:2147483647;display:block`;
    document.body.appendChild(el);
  }, { n, w, h });
  await page.screenshot({ path: join(process.argv[2], `photo-${n}-${shape}.png`), clip: { x: 0, y: 0, width: w, height: h } });
}
await browser.close(); server.close();
