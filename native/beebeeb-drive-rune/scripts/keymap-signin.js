// keymap-signin.js: the gate keys of the sign-in group (the board's `.au-mac` window: DW-A01 to
// DW-A50). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
//
// Every key is a class name and a running number in document order, so the native side counts
// the same way (`ui::au::Keys`): the 2nd `.kit-btn` of a frame is `btn-1`, on the board and in Rune.
// Boxes carry `css` (radius, shadow, border, padding, gap, opacity): the gate ignores it, the
// person building the frame reads the measurements from the golden instead of estimating them.
const auMac = q('.au-mac');
if (auMac) {
  const css = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (parseFloat(cs.borderTopWidth) || parseFloat(cs.borderRightWidth) || parseFloat(cs.borderBottomWidth)) out.border = `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth} ${cs.borderTopColor}`;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (cs.opacity !== '1') out.opacity = cs.opacity;
    return { css: out };
  };
  const shown = (el) => !!el && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden';
  const box = (key, el) => { if (shown(el)) api.box(key, el, css(el)); };
  const text = (key, el) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { lh: getComputedStyle(el).lineHeight, align: getComputedStyle(el).textAlign }); };
  const icon = (key, el) => { if (shown(el)) api.icon(key, el); };
  const each = (sel, root, f) => qa(sel, root).forEach(f);
  // The lockup: the mark, "beebeeb.io" and the product name under it.
  const lockup = (prefix, root) => {
    const svg = q(':scope > .inline-flex.items-center > .inline-flex.flex-none > svg', root);
    if (!svg) return;
    const words = svg.parentElement.nextElementSibling;
    api.icon(`${prefix}-mark`, svg, { mark: true });
    text(`${prefix}-word`, q('.font-bold', words));
    text(`${prefix}-product`, q('.font-mono', words));
  };

  // ---- The brand panel ----
  const side = q('.au-mac-side', auMac);
  box('au-side', side);
  lockup('side', side);
  text('side-tagline', q(':scope > p', side));
  each('ul li', side, (li, i) => { icon(`fact-${i}-icon`, q('svg', li)); text(`fact-${i}`, q('span', li)); });

  // ---- The column ----
  const main = q('.au-mac-main', auMac);
  box('au-main', main);
  const col = q('.au-mac-col', main);
  lockup('col', col);
  const back = q(':scope > .t-sub.au-dim.au-row', col);
  if (back) { icon('back-icon', q('svg', back)); api.textNode('back', back); }
  text('title', q('.au-h h1', col));
  text('subtitle', q('.au-h p', col));

  each('.kit-btn', col, (b, i) => {
    box(`btn-${i}`, b);
    icon(`btn-${i}-icon`, q('svg', b));
    text(`btn-${i}-label`, q('span', b));
  });
  each('.kit-banner', col, (b, i) => {
    box(`banner-${i}`, b);
    icon(`banner-${i}-icon`, q('.kit-banner-icon svg', b));
    text(`banner-${i}-title`, q('.kit-banner-text > b', b));
    text(`banner-${i}-text`, q('.kit-banner-text > span', b));
  });
  each('.kit-fieldwrap', col, (w, i) => {
    text(`field-${i}-label`, q('.kit-field-outer', w));
    const f = q('.kit-field', w);
    box(`field-${i}`, f);
    icon(`field-${i}-icon`, q('.kit-field-icon svg', f));
    text(`field-${i}-inner-label`, q('.kit-field-label', f));
    const value = q('.kit-field-value', f);
    if (shown(value) && value.textContent.trim()) api.textNode(`field-${i}-value`, value, { secure: value.classList.contains('is-secure') });
    box(`field-${i}-caret`, q('.kit-caret', f));
    const act = q('.kit-field-act', f);
    box(`field-${i}-act`, act);
    icon(`field-${i}-act-icon`, act && q('svg', act));
    text(`field-${i}-msg`, q('.kit-field-msg', w));
    const strength = q('.kit-strength', w);
    if (strength) {
      each(':scope > i', strength, (bar, j) => box(`field-${i}-strength-${j}`, bar));
      text(`field-${i}-strength`, q('span', strength));
    }
  });
  each('.au-card', col, (c, i) => {
    box(`card-${i}`, c);
    const av = q('.kit-avatar', c);
    if (av) { box(`card-${i}-avatar`, av); api.textNode(`card-${i}-initials`, av); }
    icon(`card-${i}-icon`, q(':scope > .au-row > svg', c));
    text(`card-${i}-title`, q('.t-head', c));
    each('.t-cap', c, (t, j) => text(`card-${i}-line-${j}`, t));
    text(`card-${i}-text`, q(':scope > p', c));
  });
  each('.au-or', col, (o, i) => {
    each(':scope > i', o, (line, j) => box(`or-${i}-line-${j}`, line));
    api.textNode(`or-${i}`, o);
  });
  each('.au-link', col, (l, i) => {
    icon(`link-${i}-icon`, q(':scope > svg', l));
    api.textNode(`link-${i}`, l);
  });
  // A paragraph that stands by itself in the column (not the heading's, not a card's, not a field's).
  qa('p, b.t-head', col).filter((p) => !p.closest('.au-h, .au-card, .kit-fieldwrap, .kit-banner')).forEach((p, i) => text(`note-${i}`, p));
  each('.au-hero', col, (h) => {
    box('hero', h);
    const tile = q('.kit-tile', h);
    if (tile) { box('hero-tile', tile); icon('hero-icon', q('svg', tile)); }
    const ring = q('.kit-ring', h);
    if (ring) { box('hero-ring', ring); icon('hero-icon', q(':scope > span:not(.kit-ring) svg', h)); }
  });
  each(':scope > .au-row > .kit-avatar', col, (av) => {
    box('who-avatar', av); api.textNode('who-initials', av);
    text('who-email', av.nextElementSibling);
  });
  const qr = q('.au-qr', col);
  if (qr) { box('qr', qr); box('qr-code', q('svg', qr)); }
  text('code', q('.au-code', col));
  const status = q('.au-stack > .au-row', col);
  if (status) {
    box('status-ring', q('.kit-ring', status));
    const exp = q(':scope > .t-sub', status);
    if (exp) { api.textNode('status-expires', exp); text('status-time', q('.au-mono', exp)); }
    const pill = q('.kit-pill', status);
    if (pill) { box('status-pill', pill); box('status-dot', q('.kit-dot', pill)); api.textNode('status-pill-text', pill); }
  }
  each('.kit-list-body', col, (l, i) => box(`list-${i}`, l));
  each('.kit-item', col, (it, i) => {
    box(`item-${i}`, it);
    icon(`item-${i}-icon`, q('.kit-item-icon svg', it));
    text(`item-${i}-title`, q('.kit-item-title', it));
    text(`item-${i}-sub`, q('.kit-item-sub', it));
    text(`item-${i}-value`, q('.kit-item-value', it));
    icon(`item-${i}-chev`, q('.kit-item-chev svg', it));
  });
  each('.au-digit', col, (d, i) => { box(`digit-${i}`, d); if (d.textContent.trim()) api.textNode(`digit-${i}-text`, d); });
  each('.au-word', col, (w, i) => {
    box(`word-${i}`, w);
    text(`word-${i}-n`, q('span', w));
    const em = q('em', w);
    if (em && em.textContent.trim()) text(`word-${i}-text`, em);
    box(`word-${i}-caret`, q('.kit-caret', w));
  });
  // The support address row of DW-A46 (an icon beside a link).
  icon('mail-icon', q('.au-stack > .t-sub.au-row > svg', col));

  // ---- Create an account (DW-A31 to DW-A44) ----
  // The progress line: six bars, "2 of 6" and the step's name.
  const steps = q('.au-steps', col);
  if (steps) {
    each(':scope > b > i', steps, (bar, i) => box(`step-${i}`, bar));
    text('steps-count', q(':scope > .au-mono', steps));
    text('steps-name', q(':scope > .au-dim', steps));
  }
  // A checkbox with its sentence (the terms, "I have saved my recovery phrase offline.").
  each('label > button[role="checkbox"]', col, (b, i) => {
    box(`check-${i}`, b);
    icon(`check-${i}-icon`, q('svg', b));
    text(`check-${i}-label`, b.nextElementSibling);
  });
  // "You can ask for a new code in 0:42".
  const resend = q('.au-stack.is-tight > span:not(.kit-go)', col);
  if (resend && q(':scope > .au-mono', resend)) { api.textNode('resend', resend); text('resend-time', q(':scope > .au-mono', resend)); }
  // The suggestions under the word being typed (DW-A40, DW-A41).
  const suggest = q('.au-row.is-between > .au-row', col);
  if (suggest && q('.kit-chip', suggest)) {
    text('suggest-label', q(':scope > span:not(.kit-chip)', suggest));
    each(':scope > .kit-chip', suggest, (c, i) => { box(`chip-${i}`, c); api.textNode(`chip-${i}-text`, c); });
  }
  // The "Next" card of DW-A44: a tile, a caption and the product's step. Only a card built that way: the
  // first run's own cards (DW-0204, DW-0206) are keyed in keymap-signin-setup.js.
  qa(':scope > .kit-card', col).filter((c) => q(':scope > .kit-card-body > .au-row > .kit-tile', c)).forEach((c, i) => {
    box(`next-${i}`, c);
    const tile = q('.kit-tile', c);
    box(`next-${i}-tile`, tile);
    icon(`next-${i}-icon`, q('svg', tile));
    text(`next-${i}-cap`, q('.t-cap', c));
    text(`next-${i}-title`, q('.t-head', c));
  });
}
