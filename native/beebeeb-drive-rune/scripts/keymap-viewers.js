// keymap-viewers.js: the gate keys of the open-file group (DW-1501 to DW-1527: the viewers, their
// states, text and Markdown editing). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
//
// Scoped to this group's frames by the board's own frame id, so no other group's golden gains a key.
// Keys are class names and a running number in document order; the native side counts the same way.
// Boxes carry `css` (radius, shadow, border, padding, gap, opacity): the gate ignores it, the person
// building the frame reads the measurements from the golden.
const vwFrame = (win.closest('[id^="DW-"]') || {}).id || '';
// DW-1520 to DW-1522 are the Files window with New's menu or the New file dialog over it: keyed
// below ("New file"), by the Files window's names; the open-file sections do not apply to them.
const vwNewFile = /^DW-152[0-2]$/.test(vwFrame);
if (/^DW-15(0[1-6]|1[2-9]|2[0-7])$/.test(vwFrame) && !vwNewFile) {
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
  const tg = q('.mk-tgroup', tb);
  if (tg) box('view-toggle-dim', tg);

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

// ---- New file (DW-1520 to DW-1522): the Files window with New's menu, then the dialog that picks the
// type (DW-1521) and names the file (DW-1522). The menu by the Files window's names (keymap-files.js:
// `menu`, `menu-item-<n>`); the dialog's card by `ui::dialog`'s (`scrim`, `dlg`, `dlg-title`, `dlg-sub`,
// `dlg-foot`), its buttons and fields by the Files dialogs' (`btn-<n>`, `field-<n>`).
if (vwNewFile) {
  // The helpers of the open-file block above, in this block's own scope.
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
  const shown = (el) => !!el && el.getClientRects().length > 0;
  const box = (key, el, extra = {}) => { if (shown(el)) api.box(key, el, { ...css(el), ...extra }); };
  const tcss = (el) => { const cs = getComputedStyle(el); return { lh: cs.lineHeight, align: cs.textAlign, italic: cs.fontStyle === 'italic' }; };
  const text = (key, el, extra = {}) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { ...tcss(el), ...extra }); };
  qa('.wn-menu').forEach((m, mi) => {
    const mk = mi ? `menu-${mi}` : 'menu';
    api.box(mk, m);
    qa('.wn-mi', m).forEach((it, i) => {
      const k = `${mk}-item-${i}`;
      api.box(k, it, { hot: it.classList.contains('is-hot') });
      api.icon(`${k}-check`, q('.ck svg', it));
      api.text(`${k}-label`, q('.lb', it));
      if (q('.sc', it)) api.textNode(`${k}-shortcut`, q('.sc', it));
    });
    qa('.wn-msep', m).forEach((s, i) => api.box(`${mk}-sep-${i}`, s));
  });
  const nf = q('.wn-dlg');
  if (nf) {
    box('scrim', q('.mk-scrim'));
    box('dlg', nf);
    text('dlg-title', q('.wn-dlg-head .t-title', nf));
    text('dlg-sub', q('.wn-dlg-head .t-sub', nf));
    text('dlg-label', q('.wn-dlg-body > .t-label', nf));
    // The five types (DW-1521): a tile each, the outline of a document with its extension, the word.
    qa('.wn-dlg-body .grid-cols-5 > div', nf).forEach((tile, i) => {
      box(`type-${i}`, tile);
      const doc = q(':scope > span:first-child', tile);
      box(`type-${i}-doc`, doc);
      if (doc) text(`type-${i}-ext`, q('.font-mono', doc));
      text(`type-${i}-label`, q(':scope > .t-sub', tile));
    });
    // The name and the extension, each its own field (DW-1522); the selected name, its caret.
    qa('.wn-dlg-body .kit-field', nf).forEach((f, i) => {
      box(`field-${i}`, f, { focus: f.classList.contains('is-focus') });
      text(`field-${i}-value`, q('.kit-field-value > span', f) || q('.kit-field-value', f));
      box(`field-${i}-caret`, q('.kit-caret', f));
    });
    // The extensions offered (DW-1522): a chip each, the chosen one dark.
    qa('.wn-dlg-body .kit-chip', nf).forEach((c, i) => {
      box(`chip-${i}`, c, { on: c.classList.contains('is-active') });
      api.textNode(`chip-${i}-label`, c, tcss(c));
    });
    box('dlg-foot', q('.wn-dlg-foot', nf));
    qa('.wn-dlg-foot button.kit-btn', nf).forEach((b, i) => {
      box(`btn-${i}`, b);
      text(`btn-${i}-label`, q('span', b));
    });
  }
}
