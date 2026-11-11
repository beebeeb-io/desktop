// keymap-viewers.js: the gate keys of the open-file group (DW-1501 to DW-1527: the viewers, their
// states, text and Markdown editing). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
//
// Scoped to this group's frames by the board's own frame id, so no other group's golden gains a key.
// Keys are class names and a running number in document order; the native side counts the same way.
// Boxes carry `css` (radius, shadow, border, padding, gap, opacity): the gate ignores it, the person
// building the frame reads the measurements from the golden.
const vwFrame = (win.closest('[id^="DW-"]') || {}).id || '';
if (/^DW-15(0[1-6]|1[2-9]|2[0-7])$/.test(vwFrame)) {
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
    box('dlg-foot', q('.wn-dlg-foot', dlg));
    qa('.wn-dlg-foot button.kit-btn', dlg).forEach((b, i) => button(`dlg-btn-${i}`, b));
  }
}
