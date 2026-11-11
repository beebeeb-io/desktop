// keymap-tray.js: the gate keys of the tray group, the b's flyout (the board's `.mk-pop` in a
// `.wn-fly` crop: DW-1202 to DW-1222). Runs after keymap.js, in the same scope (`win`, `api`,
// `q`, `qa`).
//
// The flyout is a window of its own (340 x 520); the wallpaper and the taskbar around it on the
// board are Windows' (`.kit-os`), so `capture-golden.mjs` exports the flyout's own box for these
// frames and every coordinate is relative to it.
//
// Keys are class names and running numbers in document order (`row-3-name` is the name of the
// 4th `.mk-pr`), never positions or strings. Boxes carry `css` (radius, border, padding, gap,
// opacity, mask): the gate ignores it; whoever builds a frame reads the measurements from it.
const fly = q('.wn-fly .mk-pop');
if (fly) {
  const css = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (parseFloat(cs.borderTopWidth) || parseFloat(cs.borderRightWidth) || parseFloat(cs.borderBottomWidth)) out.border = `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth} ${cs.borderTopColor}`;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (cs.opacity !== '1') out.opacity = cs.opacity;
    if (cs.backgroundImage !== 'none') out.image = cs.backgroundImage;
    if (cs.maskImage && cs.maskImage !== 'none') out.mask = cs.maskImage;
    return { css: out };
  };
  const shown = (el) => !!el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden';
  const box = (key, el, extra = {}) => { if (shown(el)) api.box(key, el, { ...css(el), ...extra }); };
  const text = (key, el) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { lh: getComputedStyle(el).lineHeight, align: getComputedStyle(el).textAlign }); };
  const icon = (key, el, extra = {}) => { if (shown(el)) api.icon(key, el, extra); };
  // The b template glyph is a `<use href="#mk-b">`: the rounded square with the b cut out of it
  // by a mask. Its shapes are the symbol's and the mask's, read from the page's sprite.
  const bGlyph = (key, svg) => {
    if (!shown(svg)) return;
    const shapes = ['#mk-b', '#mk-bmask'].flatMap((id) => [...document.querySelectorAll(`${id} path, ${id} rect[rx]`)]);
    const d = shapes.map((s) => s.tagName === 'path' ? s.getAttribute('d')
      : `${s.tagName}:${[...s.attributes].filter((a) => !/^(fill|stroke|class|style)/.test(a.name)).map((a) => `${a.name}=${a.value}`).join(',')}`);
    api.icon(key, svg, { icon: d, filled: true, state: 'template' });
  };
  // A button of the kit (`.kit-btn`): its box and its label.
  const button = (key, b) => { box(key, b); text(`${key}-label`, b && q('span', b)); };
  // A progress bar (`.mk-bar`): the track and what is filled.
  const bar = (key, b) => { box(key, b, { kind_: b ? b.className : '' }); box(`${key}-fill`, b && q('i', b)); };

  box('fly', fly);

  // ---- The status block ----
  const st = q('.mk-pst', fly);
  box('st', st);
  const tile = q('.mk-pst-ic', st);
  box('st-tile', tile, { tone: tile ? tile.className.replace('mk-pst-ic', '').trim() : '' });
  icon('st-icon', tile && q('svg', tile));
  const tx = q('.mk-pst-tx', st);
  text('st-title', q(':scope > b', tx));
  text('st-line', q(':scope > span:not(.font-mono):not(.mk-skel)', tx));
  text('st-code', q(':scope > .font-mono', tx));
  qa('.mk-skel', st).forEach((s, i) => box(`st-skel-${i}`, s));
  button('st-btn', q('.kit-btn', st));
  bar('st-bar', q('.mk-bar', st));
  const speed = q('[title="Upload speed"]', st);
  if (speed) { icon('st-speed-icon', q('svg', speed)); api.textNode('st-speed', speed, { lh: getComputedStyle(speed).lineHeight }); }

  // ---- Recent changes ----
  const label = q('.mk-plabel', fly);
  box('label', label);
  if (label) { api.textNode('label-title', label); text('label-count', q('span', label)); }
  const list = q('.mk-plist', fly);
  box('list', list, { fade: list && list.classList.contains('is-end') ? 'top' : 'bottom' });
  qa('.mk-pr', list).forEach((row, i) => {
    const k = `row-${i}`;
    box(k, row, { title: row.getAttribute('title') || '' });
    const ft = q('.mk-ftile', row);
    box(`${k}-tile`, ft, { thumb: ft.className.replace('mk-ftile', '').trim() });
    icon(`${k}-icon`, q('svg', ft));
    const rtx = q('.mk-pr-tx', row);
    text(`${k}-name`, q(':scope > b', rtx));
    const sub = q(':scope > span:not(.mk-bar)', rtx);
    if (sub) {
      icon(`${k}-sub-icon`, q(':scope > svg', sub));
      text(`${k}-sub`, q(':scope > span', sub) || sub);
    }
    bar(`${k}-bar`, q('.mk-bar', rtx));
    text(`${k}-time`, q('.mk-pr-t', row));
  });
  // The list with nothing to show: a tile, a title, a sentence.
  const empty = q(':scope > .flex.flex-col', list);
  if (empty) {
    box('empty-tile', q('.kit-tile', empty));
    icon('empty-icon', q('.kit-tile svg', empty));
    text('empty-title', q('.t-head', empty));
    text('empty-text', q('.t-sub', empty));
  }
  // The end of the list: one line, the way to the rest, and the scroll thumb.
  const end = q('.mk-pend', list);
  if (end) { text('end-text', q('small', end)); button('end-btn', q('.kit-btn', end)); }
  box('scroll-thumb', q('.mk-scroll', list));

  // ---- The three buttons ----
  const foot = q('.mk-pfoot', fly);
  box('foot', foot);
  qa('.mk-pbtn', foot).forEach((b, i) => {
    const k = `foot-${i}`;
    box(k, b, { off: b.classList.contains('is-off') });
    const svg = q('svg', b);
    if (svg && q('use', svg)) bGlyph(`${k}-icon`, svg); else icon(`${k}-icon`, svg);
    api.textNode(`${k}-label`, b, { lh: getComputedStyle(b).lineHeight });
  });
}
