#!/usr/bin/env node
// Clean golden capture + geometry export with Rune-matching keys.
// Keys map to Rune's element.key values for the geometry check.
import { chromium } from 'playwright';
import { writeFileSync, mkdirSync, readFileSync } from 'fs';
import { join } from 'path';
import { createServer } from 'http';

const outDir = process.argv[2] || '/tmp/board-extract/golden';
mkdirSync(outDir, { recursive: true });

const boardDir = '/home/guus/code/beebeeb.io/design/desktop-v2';
const server = createServer((req, res) => {
  const path = boardDir + (req.url === '/' ? '/board.html' : req.url);
  try {
    const data = readFileSync(path);
    const ct = path.endsWith('.html') ? 'text/html; charset=utf-8'
             : path.endsWith('.js') ? 'text/javascript; charset=utf-8'
             : path.endsWith('.css') ? 'text/css; charset=utf-8'
             : path.endsWith('.woff2') ? 'font/woff2'
             : 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': ct });
    res.end(data);
  } catch { res.writeHead(404); res.end('Not found'); }
});
await new Promise(resolve => server.listen(9881, resolve));

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1600, height: 1200 }, deviceScaleFactor: 2 });
await page.goto('http://127.0.0.1:9881/board.html', { waitUntil: 'networkidle', timeout: 60000 });

// Navigate to Windows > DW-0401
for (const tab of await page.$$('.kit-switch a, .kit-tabs a')) {
  if ((await tab.textContent())?.trim() === 'Windows') { await tab.click(); await page.waitForTimeout(2000); break; }
}
const link = await page.$('a:has-text("DW-0401")');
if (link) { await link.click(); await page.waitForTimeout(3000); }

// Hide every sticky/fixed element outside the frame
await page.evaluate(() => {
  for (const el of document.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if ((cs.position === 'sticky' || cs.position === 'fixed') && !el.closest('#DW-0401 .kit-screen'))
      el.style.display = 'none';
  }
  const cap = document.querySelector('#DW-0401 > figcaption');
  if (cap) cap.style.display = 'none';
});
await page.waitForTimeout(300);

// Screenshot the window
const winEl = await page.$('#DW-0401 .kit-screen');
if (!winEl) { console.error('not found'); process.exit(1); }
await winEl.screenshot({ path: join(outDir, 'DW-0401-board.png') });

// Verify size
const { execSync } = await import('child_process');
const dims = execSync(`python3 -c "from PIL import Image; i=Image.open('${join(outDir, 'DW-0401-board.png')}'); print(f'{i.size[0]}x{i.size[1]}')"`, { encoding: 'utf-8' }).trim();
console.log(`Screenshot: ${dims}`);
if (dims !== '2560x1600') {
  execSync(`python3 -c "
from PIL import Image
img = Image.open('${join(outDir, 'DW-0401-board.png')}')
img = img.crop((0, 0, 2560, 1600))
img.save('${join(outDir, 'DW-0401-board.png')}')
print(f'Cropped to {img.size}')
"`, { stdio: 'inherit' });
}

// Assert UTF-8
const ft = await winEl.evaluate(el => el.innerText);
if (ft.includes('â€"') || ft.includes('Â·')) { console.error('mojibake'); process.exit(1); }
if (!ft.includes('·')) { console.error('missing ·'); process.exit(1); }
console.log('UTF-8 OK');

// ---- Geometry export ----
const geo = await page.evaluate(() => {
  const win = document.querySelector('#DW-0401 .kit-screen');
  const wr = win.getBoundingClientRect();
  const r = el => {
    const b = el.getBoundingClientRect();
    return {
      x: Math.round((b.x - wr.x) * 10) / 10,
      y: Math.round((b.y - wr.y) * 10) / 10,
      w: Math.round(b.width * 10) / 10,
      h: Math.round(b.height * 10) / 10,
    };
  };
  const s = el => {
    const cs = getComputedStyle(el);
    return {
      fontSize: parseFloat(cs.fontSize),
      fontWeight: parseInt(cs.fontWeight),
      fontFamily: cs.fontFamily?.split(',')[0]?.trim().replace(/['"]/g, ''),
      color: cs.color,
      bg: cs.backgroundColor === 'rgba(0, 0, 0, 0)' ? undefined : cs.backgroundColor,
    };
  };

  const out = {};
  const add = (k, el, extra) => { if (el) out[k] = { ...r(el), ...s(el), ...extra }; };

  // Helper: first text node matching str in container, optionally filtered by x range
  const findText = (root, str, minX, maxX) => {
    const tw = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    while (tw.nextNode()) {
      if (tw.currentNode.textContent.trim() === str) {
        const el = tw.currentNode.parentElement;
        const pos = r(el);
        if (minX !== undefined && pos.x < minX) continue;
        if (maxX !== undefined && pos.x > maxX) continue;
        return el;
      }
    }
    return null;
  };

  // ---- Caption bar (kit-wincap) ----
  const wincap = win.querySelector('.kit-wincap');
  add('caption-bar', wincap);

  // Brand block: "beebeeb.io" + "DRIVE"
  const bMark = win.querySelector('.kit-wincap svg, .mk-side svg');
  if (bMark) add('b-mark', bMark.closest('span') || bMark.parentElement);
  add('drive-label', findText(win, 'DRIVE') || findText(win, 'Drive'));

  // ---- Sidebar (mk-side) ----
  const sidebar = win.querySelector('.mk-side, aside');
  add('sidebar', sidebar);

  // Sidebar section text labels (use the text element directly, not the collapsed parent)
  for (const label of ['Files', 'Shared', 'Photos', 'Backups', 'Trash']) {
    const el = findText(sidebar, label, 0, 220);
    if (el) add(label, el, { text: label });
  }
  // Badges
  add('Shared-badge', findText(sidebar, '1', 170, 220));
  add('Trash-badge', findText(sidebar, '12', 170, 220));

  // THIS PC
  const thisPc = findText(sidebar, 'This PC') || findText(sidebar, 'THIS PC');
  add('this-pc-label', thisPc, { text: 'THIS PC' });

  add('Sync history', findText(sidebar, 'Sync history'), { text: 'Sync history' });
  const ao = findText(sidebar, 'Available offline');
  add('Available offline', ao, { text: 'Available offline' });
  add('Available offline-badge', findText(sidebar, '3', 185, 220));

  // Storage
  add('storage-label', findText(sidebar, 'Storage'));
  // Account
  add('account-name', findText(sidebar, 'Anna Visser'));
  add('account-email', findText(sidebar, 'anna@northwind.studio'));

  // ---- Toolbar ----
  const toolbar = win.querySelector('.mk-main') || win.querySelector('.mk-body');
  add('toolbar', win.querySelector('.mk-toolbar') || (() => {
    // toolbar is the row between the caption and the column headers
    const body = win.querySelector('.mk-main, .mk-body');
    if (body) {
      for (const ch of body.children) {
        const cr = r(ch);
        if (cr.y < 50 && cr.h < 60 && cr.h > 30) return ch;
      }
    }
    return null;
  })());

  add('folder-title', findText(win, 'Files', 250, 400));
  add('item-count', findText(win, '11 items'));
  add('kbd-hint', findText(win, 'Ctrl+F'));
  add('search-placeholder', findText(win, 'Search names'));

  // + New button
  const newText = findText(win, 'New', 900, 1100);
  if (newText) {
    let btn = newText;
    for (let i = 0; i < 3; i++) {
      const p = btn.parentElement;
      if (p) { btn = p; const cs = getComputedStyle(btn); if (cs.backgroundColor !== 'rgba(0, 0, 0, 0)') break; }
    }
    add('new-btn', btn);
  }

  // ---- Column headers ----
  add('col-name', findText(win, 'Name', 230, 350));
  add('col-date', findText(win, 'Date changed'));
  add('col-size', findText(win, 'Size', 700, 800));
  add('col-shared', findText(win, 'Shared', 820, 950));
  add('col-pc', findText(win, 'PC', 930, 960));

  // ---- File rows ----
  const names = ['Backups', 'Documents', 'Invoices 2026', 'Lisbon trip', 'Shoot selects',
    'Budget 2026.xlsx', 'Contract draft.pdf', 'Harbour.heic',
    'Northwind brand guide.pdf', 'Q4 plan.key', 'Studio rent 2026.pdf'];
  for (let i = 0; i < names.length; i++) {
    const nameEl = findText(win, names[i], 250, 500);
    if (!nameEl) continue;
    add(`row-${i}-name`, nameEl, { text: names[i] });
    // Walk up to the row div (mk-row)
    let row = nameEl;
    for (let j = 0; j < 5; j++) {
      if (row.parentElement) {
        row = row.parentElement;
        const rr = r(row);
        if (rr.w > 500 && rr.h >= 28 && rr.h <= 40) break;
      }
    }
    add(`row-${i}`, row);
  }

  // ---- Detail pane ----
  add('detail-name', findText(win, 'Contract draft.pdf', 950, 1280));
  add('encrypted-text', findText(win, 'End-to-end encrypted'));
  // The encrypted badge container
  const e2e = findText(win, 'End-to-end encrypted');
  if (e2e) {
    let badge = e2e;
    for (let i = 0; i < 3; i++) {
      const p = badge.parentElement;
      if (p) { badge = p; const cs = getComputedStyle(badge); if (cs.backgroundColor !== 'rgba(0, 0, 0, 0)') break; }
    }
    add('encrypted-badge', badge);
  }

  // Detail metadata labels
  for (const label of ['Changed', 'Changed on', 'Added', 'Where', 'Version history']) {
    add(`detail-${label.toLowerCase().replace(/ /g, '-')}`, findText(win, label, 990, 1120));
  }
  add('detail-shared', findText(win, 'Shared', 990, 1120));

  // Available offline (detail)
  add('offline-label', findText(win, 'Available offline', 990, 1280));

  // Share... / Version history buttons in detail
  add('share-btn', findText(win, 'Share…', 990, 1280));
  add('versions-btn', findText(win, 'Version history', 1100, 1280));

  // ---- Status bar ----
  add('status-text', findText(win, 'Up to date'));
  // Right side counts
  const tw = document.createTreeWalker(win, NodeFilter.SHOW_TEXT);
  while (tw.nextNode()) {
    const t = tw.currentNode.textContent.trim();
    if (t.includes('4 on this PC')) {
      add('status-details', tw.currentNode.parentElement);
      break;
    }
  }
  // Status bar container
  const stText = findText(win, 'Up to date');
  if (stText) {
    let bar = stText;
    for (let i = 0; i < 4; i++) {
      if (bar.parentElement) {
        bar = bar.parentElement;
        const br = r(bar);
        if (br.w > 800 && br.h < 40 && br.h > 20) { add('status-bar', bar); break; }
      }
    }
  }

  // Find the detail pane container
  const dn = findText(win, 'Contract draft.pdf', 950, 1280);
  if (dn) {
    let pane = dn;
    for (let i = 0; i < 8; i++) {
      if (pane.parentElement) {
        pane = pane.parentElement;
        const pr = r(pane);
        if (pr.h > 500 && pr.w > 200 && pr.w < 400) { add('detail-pane', pane); break; }
      }
    }
  }

  return {
    width: Math.round(wr.width),
    height: Math.round(wr.height),
    elementCount: Object.keys(out).length,
    elements: out,
  };
});

writeFileSync(join(outDir, 'DW-0401.json'), JSON.stringify(geo, null, 2));
console.log(`Geometry: ${geo.elementCount} elements`);

// Summary
for (const [k, v] of Object.entries(geo.elements).sort((a, b) => a[0].localeCompare(b[0]))) {
  const t = v.text ? ` "${v.text.substring(0, 25)}"` : '';
  console.log(`  ${k.padEnd(30)} x=${String(v.x).padEnd(6)} y=${String(v.y).padEnd(6)} w=${String(v.w).padEnd(6)} h=${String(v.h).padEnd(5)} ${v.fontSize}px/${v.fontWeight} ${v.fontFamily}${t}`);
}

server.close();
await browser.close();
console.log('Done');
