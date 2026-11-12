// keymap-viewers.js: the gate keys of the open-file group (DW-1501 to DW-1527: the viewers, their
// states, text and Markdown editing). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
//
// Scoped to this group's frames by the board's own frame id, so no other group's golden gains a key.
// Keys are class names and a running number in document order; the native side counts the same way.
// Boxes carry `css` (radius, shadow, border, padding, gap, opacity): the gate ignores it, the person
// building the frame reads the measurements from the golden.
const vwFrame = (win.closest('[id^="DW-"]') || {}).id || '';
if (/^DW-15(0[1-6]|1[2-9]|2[0-7])$/.test(vwFrame)) {
  // Guard: the base keymap's `.mk-tgroup` loop writes keys named `undefined` and `undefined-icon`
  // when the toolbar's group has 3 buttons (the PDF's zoom control, DW-1505 and DW-1506). Those
  // keys are already in `out` by the time this file runs; prevent future undefined-key writes.
  const _origBox = api.box.bind(api), _origIcon = api.icon.bind(api), _origText = api.text.bind(api);
  api.box = (key, ...args) => { if (key === undefined) return; _origBox(key, ...args); };
  api.icon = (key, ...args) => { if (key === undefined) return; _origIcon(key, ...args); };
  api.text = (key, ...args) => { if (key === undefined) return; _origText(key, ...args); };
  const css = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (parseFloat(cs.borderTopWidth) || parseFloat(cs.borderRightWidth) || parseFloat(cs.borderBottomWidth) || parseFloat(cs.borderLeftWidth)) out.border = `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth} ${cs.borderTopColor}`;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (cs.opacity !== '1') out.opacity = cs.opacity;
    return { css: out };
  };
  // A computed colour as sRGB bytes (the page writes oklch and color-mix).
  const probe = document.createElement('canvas'); probe.width = probe.height = 1;
  const pctx = probe.getContext('2d', { willReadFrequently: true });
  const ink = (el) => { pctx.clearRect(0, 0, 1, 1); pctx.fillStyle = '#000'; pctx.fillStyle = getComputedStyle(el).color; pctx.fillRect(0, 0, 1, 1); return [...pctx.getImageData(0, 0, 1, 1).data]; };
  const shown = (el) => !!el && el.getClientRects().length > 0;
  const box = (key, el, extra = {}) => { if (shown(el)) api.box(key, el, { ...css(el), ...extra }); };
  const tcss = (el) => { const cs = getComputedStyle(el); return { lh: cs.lineHeight, align: cs.textAlign, italic: cs.fontStyle === 'italic' }; };
  const text = (key, el, extra = {}) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { ...tcss(el), ...extra }); };
  const icon = (key, el) => { if (shown(el)) api.icon(key, el); };
  // A button of the kit: its box, its glyph, its word, and a mono shortcut after the word.
  const button = (key, b) => {
    if (!shown(b)) return;
    box(key, b);
    icon(`${key}-icon`, q(':scope > svg', b));
    const word = q(':scope > span', b);
    const hint = word && q(':scope > .font-mono', word);
    if (hint) { api.textNode(`${key}-label`, word, tcss(word)); text(`${key}-hint`, hint, { opacity: getComputedStyle(hint).opacity }); }
    else text(`${key}-label`, word);
  };

  // ---- Toolbar: the path, and the open file's own controls ----
  const tb = q('.mk-tb');
  const crumbs = q('.mk-crumbs', tb);
  if (crumbs) {
    qa(':scope > i', crumbs).forEach((c, i) => text(`crumb-${i}`, c));
    qa(':scope > svg', crumbs).forEach((s, i) => icon(`crumb-sep-${i}`, s));
    const name = q(':scope > span', crumbs);
    const inner = name && q(':scope > .inline-flex', name);
    if (inner) { api.textNode('crumb-name', inner, tcss(name)); box('crumb-dot', q(':scope > span', inner)); }
    else text('crumb-name', name);
  }
  const acts = q(':scope > span.flex.items-center', tb);
  const seg = acts && q('.kit-seg', acts);
  if (seg) {
    box('seg', seg);
    qa('[role="tab"]', seg).forEach((s, i) => { box(`seg-${i}`, s, { on: s.classList.contains('on') }); api.textNode(`seg-${i}-label`, s, tcss(s)); });
  }
  if (acts) qa('button.kit-btn', acts).forEach((b, i) => button(`tb-btn-${i}`, b));
  const tools = [...(acts ? qa('.mk-tbtn', acts) : []), ...qa(':scope > .mk-tbtn:not([title])', tb)];
  tools.forEach((b, i) => { const k = i === tools.length - 1 ? 'more-btn' : `tool-${i}`; box(k, b); icon(`${k}-icon`, q('svg', b)); });
  const pill = acts && q('.kit-pill, .kit-chip', acts);
  if (pill) { box('tb-pill', pill); text('tb-pill-label', pill); }
  // The toolbar's `.mk-tgroup`: a view toggle (list/grid, 2 buttons) or a zoom control (−, 100%, +,
  // 3 buttons). The base keymap keys the first group as view-toggle/view-list/view-grid; when the group
  // has 3 buttons (the zoom control, DW-1505 and DW-1506) key them as zoom-* and undo the base's
  // `undefined` key that falls off its 2-entry name array.
  const tg = q('.mk-tgroup', tb);
  if (tg) {
    const tgBtns = qa('.mk-tbtn', tg);
    if (tgBtns.length === 3) {
      box('zoom-group', tg);
      box('zoom-minus', tgBtns[0]); icon('zoom-minus-icon', q('svg', tgBtns[0]));
      text('zoom-pct', tgBtns[1]);
      box('zoom-plus', tgBtns[2]); icon('zoom-plus-icon', q('svg', tgBtns[2]));
    } else {
      box('view-toggle-dim', tg);
    }
  }

  // ---- A banner under the toolbar ----
  const banner = q('.mk-main > .mk-banner');
  if (banner) {
    box('banner', banner);
    icon('banner-icon', q(':scope > svg', banner));
    text('banner-text', q('.tx', banner));
    button('banner-btn', q('button', banner));
  }

  // ---- The body ----
  const body = q('.mk-main > .mk-body');
  const first = body && body.firstElementChild;
  if (first) box('viewer-body', first);
  // A state: tile, title, words, a bar, buttons.
  const tile = first && q(':scope .kit-tile', first);
  if (first && first.classList.contains('grid') && tile) {
    box('state-tile', tile); icon('state-icon', q('svg', tile));
    text('state-title', q('.t-title', first));
    // The sentence: its colour is the colour of the words (a span inside may set it), and a mono
    // run inside it (an amount) is a text of its own.
    const sub = q('.t-sub', first);
    const subMono = sub && q(':scope > .font-mono', sub);
    const subInk = sub && sub.children.length === 1 && !subMono ? sub.firstElementChild : sub;
    if (subMono) { api.textNode('state-sub', sub, tcss(sub)); text('state-sub-mono', subMono); }
    else if (sub) text('state-sub', sub, { color: ink(subInk) });
    const bar = q('.mk-bar', first);
    if (bar) { box('state-bar', bar); box('state-bar-fill', q('i', bar)); }
    qa('button.kit-btn', first).forEach((b, i) => button(`state-btn-${i}`, b));
  }
  // Text and code: one row per line, its number, and each run of one colour.
  if (first && first.classList.contains('font-mono')) {
    qa(':scope > .flex', first).forEach((line, i) => {
      box(`line-${i}`, line, { current: !!line.style.background });
      const kids = [...line.children];
      text(`ln-${i}`, kids[0]);
      qa(':scope > span', kids[1]).forEach((run, j) => {
        if (!run.textContent) return;
        api.text(`run-${i}-${j}`, run, { ...tcss(run), text: run.textContent, underline: getComputedStyle(run).textDecorationLine === 'underline' });
      });
      box(`caret-${i}`, q('.kit-caret', kids[1]));
    });
  }
  // Markdown, rendered.
  const md = first && q(':scope > .flex.flex-col', first);
  if (md && q('h1', md)) {
    box('md', md);
    text('md-h1', q('h1', md));
    qa(':scope > p', md).forEach((p, i) => text(`md-p-${i}`, p));
    qa(':scope > .t-title', md).forEach((h, i) => text(`md-h2-${i}`, h));
    qa('li', md).forEach((li, i) => {
      text(`md-li-${i}`, li);
      const code = q('code', li); if (code) box(`md-li-${i}-code`, code);
    });
  }

  // ---- Photo viewer (DW-1501, DW-1502): a dark body with the photo placeholder, the gallery
  // strip and four action buttons (Share, Offline, Details, Trash). ----
  if (/^DW-150[12]$/.test(vwFrame) && first) {
    // The photo placeholder (the board's `.mk-ph`).
    const ph = q(':scope .mk-ph', first);
    if (ph) box('photo', ph);
    // The gallery strip: a row of thumbnail placeholders.
    const strip = qa(':scope > div', first)[1]; // second child div (the strip row)
    if (strip) {
      box('gallery', strip);
      qa('.mk-ph', strip).forEach((t, i) => box(`gallery-${i}`, t, { opacity: getComputedStyle(t).opacity }));
    }
    // Four action buttons at the bottom: Share, Offline, Details, Trash.
    const actions = qa(':scope > div', first)[2]; // third child div
    if (actions) {
      qa(':scope > span', actions).forEach((a, i) => {
        box(`action-${i}`, a);
        icon(`action-${i}-icon`, q('svg', a));
        const lbl = [...a.childNodes].find(n => n.nodeType === 3 && n.textContent.trim());
        if (lbl) { const range = document.createRange(); range.selectNodeContents(lbl); api.text(`action-${i}-label`, a, { ...tcss(a), text: lbl.textContent.trim() }); }
      });
    }
  }

  // ---- Audio player (DW-1503): artwork, title, scrubber, transport, queue. ----
  if (vwFrame === 'DW-1503' && first) {
    const cols = qa(':scope > div', first);
    const player = cols[0]; // left column: artwork, title, scrubber, controls
    const queue = cols[1];  // right column: Up next list
    if (player) {
      const ph = q('.mk-ph', player);
      if (ph) box('artwork', ph);
      text('audio-title', q('.t-title', player));
      text('audio-artist', q('.t-sub', player));
      // Scrubber bar.
      const barWrap = q('.mk-bar', player) || q(':scope > div > div', player.children[2]);
      if (barWrap) { const bar = q('.mk-bar', player); if (bar) { box('scrub-track', bar); box('scrub-fill', q('i', bar)); } }
      const times = qa('.font-mono span', player);
      if (times.length >= 2) { text('time-elapsed', times[0]); text('time-remain', times[1]); }
      // Transport: previous, pause disc, next.
      const transport = [...qa(':scope > div', player)].find(d => d.classList.contains('flex') && q('.rounded-full', d));
      if (transport) {
        const btns = qa(':scope > span', transport);
        btns.forEach((b, i) => {
          const k = ['prev', 'pause', 'next'][i] || `transport-${i}`;
          box(k, b); icon(`${k}-icon`, q('svg', b));
        });
      }
      // Caption under the controls.
      const cap = q('p.t-cap', player);
      if (cap) text('audio-caption', cap);
    }
    if (queue) {
      box('queue', queue);
      // The "Up next · Studio demos" title.
      text('queue-title', q('.kit-list-title', queue));
      // Track rows: `.kit-item` with icon, title, value.
      qa('.kit-item', queue).forEach((row, i) => {
        box(`track-${i}`, row, { selected: row.classList.contains('is-selected') });
        icon(`track-${i}-icon`, q('.kit-item-icon svg', row));
        text(`track-${i}-title`, q('.kit-item-title', row));
        text(`track-${i}-value`, q('.kit-item-value', row));
      });
    }
  }

  // ---- PDF viewer (DW-1505): page thumbnails sidebar, the page on grey. ----
  if (vwFrame === 'DW-1505' && first) {
    // Thumbnail sidebar (left column).
    const thumbCol = q(':scope > div', first);
    if (thumbCol) {
      box('thumb-sidebar', thumbCol);
      qa(':scope > div', thumbCol).forEach((t, i) => {
        box(`thumb-${i}`, t);
        const page = q(':scope > span', t);
        if (page) box(`thumb-${i}-page`, page, { current: !!getComputedStyle(page).boxShadow.match(/2px/) });
        text(`thumb-${i}-num`, q('.font-mono', t));
      });
    }
    // The page area (right of the thumbs).
    const pageArea = qa(':scope > div', first)[1];
    if (pageArea) {
      box('page-area', pageArea);
      // The page itself (MPage component).
      const pg = q(':scope > div > div', pageArea) || q(':scope div', pageArea);
      if (pg && pg.children.length > 0) {
        box('page', pg);
        // Page title and sub.
        qa(':scope > div', pg).forEach((d, i) => {
          if (d.style && d.style.fontWeight) text(`page-heading-${i}`, d);
          else if (d.tagName === 'P' || getComputedStyle(d).fontSize) text(`page-text-${i}`, d);
        });
      }
    }
  }

  // ---- Find in PDF (DW-1506): the find bar, page thumbnails, the page with highlights. ----
  if (vwFrame === 'DW-1506') {
    // Find bar: lives in `.mk-bulk`.
    const findBar = q('.mk-bulk');
    if (findBar) {
      box('find-bar', findBar);
      icon('find-bar-icon', q(':scope > svg', findBar));
      const field = q('.mk-search', findBar);
      if (field) { box('find-field', field); text('find-text', q('span', field)); box('find-caret', q('.kit-caret', field)); }
      text('find-count', q('.t-sub.font-mono', findBar));
      const chevs = qa('.mk-tbtn', findBar);
      chevs.forEach((c, i) => { box(`find-nav-${i}`, c); icon(`find-nav-${i}-icon`, q('svg', c)); });
      button('find-done', q('button', findBar));
    }
    // Thumbnails and page: same structure as DW-1505.
    if (first) {
      const thumbCol = q(':scope > div', first);
      if (thumbCol) {
        box('thumb-sidebar', thumbCol);
        qa(':scope > div', thumbCol).forEach((t, i) => {
          box(`thumb-${i}`, t);
          const page = q(':scope > span', t);
          if (page) box(`thumb-${i}-page`, page, { current: !!getComputedStyle(page).boxShadow.match(/2px/) });
          text(`thumb-${i}-num`, q('.font-mono', t));
        });
      }
      const pageArea = qa(':scope > div', first)[1];
      if (pageArea) {
        box('page-area', pageArea);
        // Highlights (mark elements).
        qa('mark', pageArea).forEach((m, i) => {
          const cs = getComputedStyle(m);
          box(`mark-${i}`, m, { current: cs.background.includes('32%') || cs.backgroundColor.includes('32%') });
        });
      }
    }
  }

  // ---- CSV table (DW-1514): column headers and cells. ----
  if (vwFrame === 'DW-1514' && first) {
    const table = q('table', first);
    if (table) {
      box('csv-table', table);
      qa('th', table).forEach((th, i) => text(`csv-th-${i}`, th));
      qa('tr', q('tbody', table)).forEach((tr, ri) => {
        qa('td', tr).forEach((td, ci) => {
          const sel = !!td.style.boxShadow && td.style.boxShadow !== 'none';
          api.text(`csv-${ri}-${ci}`, td, { ...tcss(td), text: td.textContent.trim(), ...(sel ? { selected: true } : {}) });
        });
      });
    }
  }

  // ---- Archive list (DW-1515): the banner is already keyed above; the body is the file list,
  // which the base keymap keys as rows. The archive's file list has no Shared or PC columns
  // (their header spans exist but are empty), so the base keymap's `api.textNode('col-shared')`
  // and `api.textNode('col-pc')` report problems. Filter those known-absent columns. ----
  if (vwFrame === 'DW-1515' && api.problems) {
    const known = ['no text node for col-shared', 'no text node for col-pc'];
    for (let i = api.problems.length - 1; i >= 0; i--) {
      if (known.includes(api.problems[i])) api.problems.splice(i, 1);
    }
  }

  // ---- The details panel's extra line ----
  const det = q('.mk-det');
  if (det) text('detail-note', q(':scope > p.t-cap', det));

  // ---- Over the window: a toast, a dialog on its scrim ----
  const toast = q('.mk-toast .kit-toast');
  if (toast) { box('toast', toast); icon('toast-icon', q('svg', toast)); text('toast-text', q('span', toast)); }
  box('scrim', q('.mk-scrim'));
  const dlg = q('.wn-dlg');
  if (dlg) {
    box('dlg', dlg);
    box('dlg-head', q('.wn-dlg-head', dlg));
    const dt = q('.wn-dlg-alert .kit-tile', dlg);
    if (dt) { box('dlg-tile', dt); icon('dlg-icon', q('svg', dt)); }
    text('dlg-title', q('.wn-dlg-head .t-title', dlg));
    text('dlg-sub', q('.wn-dlg-head .t-sub', dlg));
    qa('.wn-dlg-body .kit-card', dlg).forEach((c, i) => {
      box(`dlg-card-${i}`, c); icon(`dlg-card-${i}-icon`, q('svg', c));
      text(`dlg-card-${i}-title`, q('.t-head', c)); text(`dlg-card-${i}-sub`, q('.t-cap', c));
    });
    text('dlg-note', q('.wn-dlg-body > p.t-cap', dlg));
    box('dlg-foot', q('.wn-dlg-foot', dlg));
    qa('.wn-dlg-foot button.kit-btn', dlg).forEach((b, i) => button(`dlg-btn-${i}`, b));
  }
}
