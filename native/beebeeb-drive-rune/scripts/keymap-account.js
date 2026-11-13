// keymap-account.js: the gate keys of the account group inside settings (the board's `.au-set`
// view, 960 x 640: DW-A51 to DW-A84a). Runs after keymap.js, in the same scope (`win`, `api`,
// `q`, `qa`, `slug`).
//
// The scheme is the settings view's (`list-<L>`, `item-<L>-<I>`, `btn-<N>`, `pill-<N>`, `dlg`),
// so the settings components give the same keys here; fields, digits and banners count as the
// sign-in group counts them (`field-<N>`, `digit-<N>`, `banner-<N>`). Every key is a class name
// and a running number in document order. Boxes carry `css` (radius, shadow, border, padding,
// gap, opacity): the gate ignores it, the person building the frame reads the board's
// measurements from the golden instead of estimating them.
const auSet = q('.au-set');
if (auSet) {
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
  const box = (key, el, extra = {}) => { if (shown(el)) api.box(key, el, { ...css(el), ...extra }); };
  const text = (key, el) => { if (shown(el) && el.textContent.trim()) api.text(key, el, { lh: getComputedStyle(el).lineHeight }); };
  const node = (key, el) => { if (shown(el) && [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) api.textNode(key, el); };
  const icon = (key, el) => { if (shown(el)) api.icon(key, el); };
  const each = (sel, root, f) => qa(sel, root).forEach(f);
  // Words the board's markup splits over several text nodes ("This " + "PC"): one inline span around
  // them (which moves nothing), so they are measured as the one string a person reads.
  const words = (key, el) => {
    if (!shown(el)) return;
    const nodes = [...el.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim());
    if (nodes.length < 2) { node(key, el); return; }
    const span = document.createElement('span');
    nodes[0].before(span);
    nodes.forEach((n) => span.appendChild(n));
    api.text(key, span);
  };

  // The words of a text node without the spaces around them (" used · renews …" after a mono span).
  const trimmed = (key, el) => {
    if (!shown(el)) return;
    const n = [...el.childNodes].find((c) => c.nodeType === 3 && c.textContent.trim());
    if (!n) return;
    const range = document.createRange();
    range.setStart(n, n.textContent.search(/\S/));
    range.setEnd(n, n.textContent.trimEnd().length);
    const span = document.createElement('span');
    range.surroundContents(span);
    api.text(key, span);
  };

  // ---- The settings sidebar, as the account kit draws it (`.kit-side`) ----
  const nav = q('.kit-side', auSet);
  box('sidebar', nav);
  each('.kit-side-item', nav, (item) => {
    const label = q('.kit-side-label', item).textContent.trim();
    box(`${label}-row`, item, { on: item.classList.contains('on') });
    icon(`${label}-icon`, q('svg', item));
    text(label, q('.kit-side-label', item));
  });
  each('.kit-side-title', nav, (tt) => node(`side-title-${slug(tt.textContent)}`, tt));
  const navFoot = q('.kit-side-foot', nav);
  box('side-foot-box', navFoot);
  node('side-foot', navFoot);

  // ---- The pane ----
  const main = q('.au-set-main', auSet);
  box('set-main', main);
  const col = q('.au-set-col', main);
  const head = q(':scope > .au-h', col);
  text('set-title', head && q('h1', head));
  text('set-sub', head && q(':scope > p', head));

  each('.kit-list', auSet, (list, L) => {
    node(`list-${L}-title`, q(':scope > .kit-list-title', list));
    box(`list-${L}-body`, q(':scope > .kit-list-body', list));
    node(`list-${L}-foot`, q(':scope > .kit-list-foot', list));
    each('.kit-item', list, (item, I) => {
      const k = `item-${L}-${I}`;
      box(k, item, { danger: item.classList.contains('is-danger'), selected: item.classList.contains('is-selected') });
      icon(`${k}-icon`, q('.kit-item-icon svg', item));
      const av = q('.kit-avatar', item);
      if (av) { box(`${k}-avatar`, av); node(`${k}-initials`, av); }
      const tile = q('.kit-tile', item);
      if (tile) { box(`${k}-tile`, tile); icon(`${k}-tile-icon`, q('svg', tile)); }
      text(`${k}-title`, q('.kit-item-title', item));
      text(`${k}-sub`, q('.kit-item-sub', item));
      text(`${k}-value`, q('.kit-item-value', item));
      icon(`${k}-chev`, q('.kit-item-chev svg', item));
      icon(`${k}-check`, q('.kit-item-check svg', item));
      const pill = q('.kit-pill', item);
      if (pill) { box(`${k}-pill`, pill); box(`${k}-pill-dot`, q('.kit-dot', pill)); words(`${k}-pill-label`, pill); }
      const pop = q('.mk-pop2', item);
      if (pop) { box(`${k}-pop`, pop); node(`${k}-pop-label`, pop); icon(`${k}-pop-caret`, q('svg', pop)); }
      const sw = q('[role="switch"] > span', item);
      if (sw) { box(`${k}-switch`, sw, { checked: sw.parentElement.getAttribute('aria-checked') === 'true' }); box(`${k}-knob`, q('span', sw)); }
      const more = q('.kit-item-more, .mk-more, .au-more', item);
      if (more) { box(`${k}-more`, more); icon(`${k}-more-icon`, q('svg', more)); }
    });
  });
  // Not a button of Windows' own dialog over the view (`.kit-os`: Windows Hello's Cancel, DW-A14): Windows draws it.
  qa('.kit-btn', auSet).filter((b) => !b.closest('.kit-os')).forEach((b, i) => {
    box(`btn-${i}`, b);
    icon(`btn-${i}-icon`, q('svg', b));
    text(`btn-${i}-label`, q('span', b));
  });
  // The product's own part of the Account page (the board's `accountExtra` slot): Drive's plan card.
  each('.kit-card', auSet, (c, i) => {
    box(`card-${i}`, c);
    const tile = q('.kit-tile', c);
    if (tile) { box(`card-${i}-tile`, tile); icon(`card-${i}-tile-icon`, q('svg', tile)); }
    text(`card-${i}-head`, q('.t-head', c));
    const sub = q('.flex-1 .t-sub', c);
    if (sub) { text(`card-${i}-mono`, q('.font-mono', sub)); trimmed(`card-${i}-sub`, sub); }
    const link = q(':scope > .t-sub', c);
    if (link) { text(`card-${i}-link`, link); icon(`card-${i}-link-icon`, q(':scope > svg', c)); }
  });
  each('.kit-banner', auSet, (bn, i) => {
    box(`banner-${i}`, bn);
    icon(`banner-${i}-icon`, q('.kit-banner-icon svg', bn));
    text(`banner-${i}-title`, q('.kit-banner-text > b', bn));
    text(`banner-${i}-text`, q('.kit-banner-text > span', bn));
  });
  each('.kit-fieldwrap', auSet, (w, i) => {
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
  each('.au-digit', auSet, (d, i) => { box(`digit-${i}`, d); node(`digit-${i}-text`, d); });
  // The n-th run of words of a paragraph that also holds a link or a mono value ("Didn't get it? " … " in " …).
  const run = (key, n) => {
    if (!n || !n.textContent.trim()) return;
    const range = document.createRange();
    range.setStart(n, n.textContent.search(/\S/));
    range.setEnd(n, n.textContent.trimEnd().length);
    const span = document.createElement('span');
    range.surroundContents(span);
    api.text(key, span);
  };
  // Links in the pane and in a dialog (`.au-link`), in document order.
  each('.au-link', auSet, (l, i) => api.text(`link-${i}`, l));
  // "Didn't get it? Send again in 42 s": the words around the link, and the countdown in mono.
  each('p.au-dim', auSet, (p, i) => {
    if (!q('.au-link', p) || !q('.au-mono', p)) return;
    [...p.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim()).forEach((n, j) => run(`resend-${i}-${j}`, n));
    text(`resend-${i}-time`, q('.au-mono', p));
  });
  // The steps of deleting the account (`.au-steps`): four bars, "1 of 4", the step's name.
  const steps = q('.au-steps', col);
  if (steps) {
    box('steps', steps);
    each(':scope > b > i', steps, (bar, j) => box(`step-${j}`, bar, { on: bar.classList.contains('on') }));
    text('steps-count', q('.au-mono', steps));
    text('steps-label', q('.au-dim', steps));
  }
  // Two-factor set-up (DW-A69): the QR code, its key and Copy key are ONE secure group. The key's
  // characters are a secret: its box is keyed (geometry, pixels), its text is not exported.
  const qr = q('.au-qr', col);
  if (qr) {
    const group = qr.closest('.au-row');
    box('secure-setup', group);
    box('qr', qr);
    box('qr-code', q('svg', qr));
    text('setup-label', q('.t-cap.au-dim', group));
    box('setup-key', q('.au-mono', group));
  }
  // The backup codes (DW-A70): ONE secure group; each code's box is keyed, its digits are not exported.
  const codes = q('.au-codes', col);
  if (codes) {
    box('secure-codes', codes);
    each(':scope > span', codes, (c, i) => { box(`code-${i}`, c); text(`code-${i}-n`, q('i', c)); });
  }
  // An empty state (`.kit-empty`).
  const empty = q('.kit-empty', col);
  if (empty) {
    box('empty', empty);
    const tile = q('.kit-tile', empty);
    box('empty-tile', tile);
    icon('empty-icon', tile && q('svg', tile));
    text('empty-title', q('.t-title', empty));
    text('empty-sub', q('p', empty));
  }
  // The menu on one passkey (`.au-menu`): its rows are words only.
  each('.au-menu', auSet, (menu, m) => {
    box(`pop-${m}`, menu);
    qa(':scope > span', menu).forEach((mi, i) => {
      box(`pop-${m}-item-${i}`, mi);
      text(`pop-${m}-item-${i}-label`, q('.au-red', mi) || mi);
    });
  });
  // The lead of a confirm's paragraph in bold ("It's your only passkey that can unlock your data.").
  text('dlg-bold', q('.wn-dlg .wn-dlg-body p > b', auSet));
  // The score ring of the security checkup, and the two lines beside it.
  const ring = q('.kit-ring', col);
  if (ring) {
    box('ring', ring);
    text('ring-label', q('.kit-ring-label b', ring));
    const beside = ring.closest('.au-row') && q('.au-h', ring.closest('.au-row'));
    if (beside) { text('ring-head', q('b', beside)); text('ring-sub', q('p', beside)); }
  }
  // A paragraph of one string that stands by itself in the column.
  qa(':scope > p, :scope > .au-stack > p', col).filter((p) => !p.children.length).forEach((p, i) => text(`note-${i}`, p));

  // ---- A dialog over the pane (`.au-scrim` + `.wn-dlg`) ----
  box('scrim', q('.au-scrim', auSet));
  const dlg = q('.wn-dlg', auSet);
  if (dlg) {
    box('dlg', dlg);
    box('dlg-body', q('.wn-dlg-body', dlg));
    text('dlg-title', q('.au-h h1', dlg));
    qa('.wn-dlg-body p', dlg).forEach((p, i) => text(`dlg-text-${i}`, p));
    box('dlg-foot', q('.wn-dlg-foot', dlg));
  }
  each('.wn-menu', auSet, (menu, m) => {
    box(`menu-${m}`, menu);
    each('.wn-mi', menu, (mi, i) => {
      box(`menu-${m}-item-${i}`, mi, { hot: mi.classList.contains('is-hot'), danger: mi.classList.contains('is-danger') });
      icon(`menu-${m}-item-${i}-icon`, q('svg', mi));
      text(`menu-${m}-item-${i}-label`, q('.lb', mi));
    });
  });
}
