// keymap.js: which board element is which gate key. Runs in the board page, called with
// `win` (the frame's .kit-screen) and `api` (box / text / textNode / icon, see
// capture-golden.mjs). The native side gives the same key to the same element
// (`.key("…")`, or the `ui::kit` helpers for text and icons).
//
// Selectors are the board's own class names, never positions or strings. A key that
// finds nothing in a frame is simply not exported for that frame.
const q = (sel, root = win) => root.querySelector(sel);
const qa = (sel, root = win) => [...root.querySelectorAll(sel)];
const slug = (s) => s.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

// ---- Caption ----
const cap = q('.kit-wincap');
api.box('caption-bar', cap);
api.icon('cap-mark', q('.kit-wincap-icon svg', cap || win), { mark: true });
api.text('cap-title', q('.kit-wincap-title'));
qa('.kit-capbtns i').forEach((b, i) => {
  const name = ['min', 'max', 'close'][i];
  api.box(`cap-${name}`, b);
  api.icon(`cap-${name}-icon`, q('svg', b));
});

// ---- Sidebar ----
const side = q('.mk-side');
if (side) {
  api.box('sidebar', side);
  const brand = q('.mk-side-brand', side);
  if (brand) {
    api.icon('brand-mark', q('svg', brand), { mark: true });
    api.text('brand-word', q('.font-bold', brand));
    api.text('drive-label', q('.font-mono', brand));
  }
  qa('.mk-side-item', side).forEach((item) => {
    const label = q('.mk-side-label', item).textContent.trim();
    api.box(`${label}-row`, item, { on: item.classList.contains('on') });
    api.icon(`${label}-icon`, q('svg', item));
    api.text(label, q('.mk-side-label', item));
    const pill = q('.mk-badge', item);
    if (pill) { api.box(`${label}-pill`, pill); api.textNode(`${label}-badge`, pill); }
    api.text(`${label}-badge`, q('.mk-side-count', item));
  });
  if (!q('.mk-set')) api.textNode('this-pc-label', q('.mk-side-title', side)); // settings keys its own titles, below
  const store = q('.mk-store', side);
  if (store) {
    const caps = qa('.t-cap span', store);
    api.text('storage-label', caps[0]);
    api.text('storage-value', caps[1]);
    api.box('meter-track', q('.kit-meter', store));
    qa('.kit-meter i', store).forEach((f, i) => api.box(`meter-fill-${i}`, f));
    api.text('storage-files', caps[2]);
    api.text('storage-photos', caps[3]);
  }
  api.text('avatar', q('.mk-acct .kit-avatar', side));
  api.text('account-name', q('.mk-acct-text b', side));
  api.text('account-email', q('.mk-acct-text span', side));
  api.box('settings-btn', q('.mk-gear', side));
  api.icon('settings-icon', q('.mk-gear svg', side));
}

// ---- Toolbar ----
const tb = q('.mk-tb');
if (tb) {
  api.box('toolbar', tb);
  const nav = qa(':scope > .inline-flex > .mk-tbtn', tb);
  ['back', 'forward'].forEach((k, i) => { api.box(k, nav[i]); api.icon(`${k}-icon`, nav[i] && q('svg', nav[i])); });
  api.text('folder-title', q('.mk-title b', tb));
  api.text('item-count', q('.mk-title > span:last-child', tb));
  const group = q('.mk-tgroup', tb);
  if (group) {
    api.box('view-toggle', group);
    qa('.mk-tbtn', group).forEach((b, i) => {
      const k = ['view-list', 'view-grid'][i];
      api.box(k, b, { on: b.classList.contains('on') });
      api.icon(`${k}-icon`, q('svg', b));
    });
  }
  const sort = q(':scope > .mk-tbtn[title="Sort"]', tb);
  api.box('sort-btn', sort, sort ? { on: sort.classList.contains('on') } : {});
  if (sort) { const s = qa('svg', sort); api.icon('sort-icon', s[0]); api.icon('sort-caret', s[1]); }
  const panel = q(':scope > .mk-tbtn[title^="Details panel"]', tb);
  api.box('panel-toggle-btn', panel, panel ? { on: panel.classList.contains('on') } : {});
  api.icon('panel-icon', panel && q('svg', panel));
  api.box('toolbar-sep', q('.mk-sep', tb));
  const nw = q('.mk-new', tb);
  if (nw) {
    api.box('new-btn', nw);
    const s = qa('svg', nw); api.icon('plus-icon', s[0]); api.icon('new-caret', s[1]);
    api.textNode('new-label', nw);
  }
  const search = q('.mk-search', tb);
  if (search) {
    api.box('search-box', search);
    api.icon('search-icon', q('svg', search));
    api.text('search-placeholder', q(':scope > span', search));
    api.box('kbd-hint-box', q('kbd', search));
    api.textNode('kbd-hint', q('kbd', search));
  }
}

// ---- Column headers ----
const thead = q('.mk-thead');
if (thead) {
  api.box('column-headers', thead);
  const cols = qa(':scope > span', thead);
  ['col-name', 'col-date', 'col-size', 'col-shared', 'col-pc'].forEach((k, i) => {
    if (!cols[i]) return;
    api.textNode(k, cols[i]);
    if (q('svg', cols[i])) api.icon('sort-arrow', q('svg', cols[i]));
  });
}

// ---- Rows ----
api.box('rows', q('.mk-rows'));
qa('.mk-rows > .mk-row').forEach((row, i) => {
  const k = `row-${i}`;
  api.box(k, row, { selected: row.classList.contains('is-sel') });
  const tile = q('.mk-ftile', row);
  if (tile) {
    api.box(`${k}-tile`, tile, { thumb: tile.classList.contains('mk-ph') });
    api.icon(`${k}-icon`, q('svg', tile));
  }
  api.text(`${k}-name`, q('.mk-name > b', row));
  api.icon(`${k}-lock`, q('.mk-name > .mk-mac svg', row));
  const cells = qa(':scope > .mk-cell', row);
  api.text(`${k}-date`, cells[0]);
  api.text(`${k}-size`, cells[1]);
  api.text(`${k}-dash`, q(':scope > .min-w-0 > .mk-cell', row));
  const link = q('.mk-link', row);
  if (link) { api.icon(`${k}-link-icon`, q('svg', link)); api.textNode(`${k}-link`, link); }
  qa('.kit-avatars .kit-avatar', row).forEach((a, j) => api.text(`${k}-avatar-${j}`, a));
  const pc = q('.mk-cell.c .mk-mac', row);
  if (pc) api.icon(`${k}-pc`, q('svg', pc), { state: slug(pc.getAttribute('title') || '') });
});

// ---- Details pane ----
const det = q('.mk-det');
if (det) {
  api.box('detail-pane', det);
  api.box('preview', q('.mk-prev', det));
  api.box('preview-page', q('.mk-page', det));
  qa('.mk-page i', det).forEach((l, i) => api.box(`preview-line-${i}`, l));
  api.text('detail-name', q('.t-head', det));
  api.text('detail-kind', q('.t-head + .t-cap', det));
  const enc = q('.mk-enc', det);
  if (enc) { api.box('encrypted-badge', enc); api.icon('lock-icon', q('svg', enc)); api.textNode('encrypted-text', enc); }
  qa('.mk-kv dt', det).forEach((dt) => {
    const k = `detail-${slug(dt.textContent)}`;
    api.text(k, dt);
    api.text(`${k}-value`, dt.nextElementSibling);
  });
  api.box('offline-card', q('.kit-list-body', det));
  api.text('offline-label', q('.kit-item-title', det));
  api.text('offline-detail', q('.kit-item-sub', det));
  const sw = q('[role="switch"] > span', det);
  if (sw) { api.box('offline-switch', sw, { checked: sw.parentElement.getAttribute('aria-checked') === 'true' }); api.box('offline-knob', q('span', sw)); }
  qa(':scope > .flex > .kit-go > button', det).forEach((b, i) => {
    const k = ['share', 'versions'][i];
    api.box(`${k}-box`, b);
    api.icon(`${k}-icon`, q('svg', b));
    api.text(`${k}-btn`, q('span', b));
  });
}

// ---- Footer ----
const foot = q('.mk-foot');
if (foot) {
  api.box('status-bar', foot);
  api.box('status-dot', q('.mk-sdot', foot));
  api.text('status-text', q('b', foot));
  api.text('status-details', q('.ml-auto', foot));
}

// ==== Lane f-shared-search-trash: Shared, Search and Trash (src/frames/{shared,search,trash}) ====
// Everything below is keyed by class, per occurrence in document order, and only outside the
// details pane (`.mk-det`): what the pane draws is keyed above, by the Files lane, under its own
// names. `scripts/check-keymap-scope.sh` proves a Files capture exports the same keys as before.
// Not in a settings frame (`.mk-set`) and not in an account window (`.au-mac`): those lanes key their
// own lists, buttons, pills and menus (below, and in keymap-signin.js) under their own names. Without
// this scope a recapture of DW-0701 exported 4 keys more and DW-A02 8 more (integration I2).
if (!q('.mk-set') && !q('.au-mac')) {
const inDet = (el) => !!el.closest('.mk-det');
// A button (`.kit-btn`): `btn-<its words>`, `-label`, `-icon`.
const btnSeen = {};
qa('.kit-btn').filter((b) => !inDet(b)).forEach((b) => {
  // The same words twice in one frame (New request in the toolbar and in the empty state): `-2` for the second.
  const base = `btn-${slug(b.textContent)}`;
  btnSeen[base] = (btnSeen[base] || 0) + 1;
  const k = btnSeen[base] > 1 ? `${base}-${btnSeen[base]}` : base;
  api.box(k, b);
  api.icon(`${k}-icon`, q('svg', b));
  api.text(`${k}-label`, q('span', b));
});
// A worded segmented control (`.kit-seg`): `seg-<n>`, each tab `seg-<n>-<its words>` and `-label`.
qa('.kit-seg').filter((seg) => !inDet(seg)).forEach((seg, n) => {
  api.box(`seg-${n}`, seg);
  qa(':scope > span', seg).forEach((tab) => {
    const k = `seg-${n}-${slug(tab.textContent)}`;
    api.box(k, tab, { on: tab.classList.contains('on') });
    api.textNode(`${k}-label`, tab);
  });
});
if (tb) {
  const seps = qa('.mk-sep', tb);
  if (seps.length > 1) api.box('toolbar-sep-2', seps[1]);
}
// A section label (`.t-label`) outside a card: `label-<its words>`.
// (the words themselves: the element's box includes its padding)
qa('.mk-body .t-label').forEach((l) => { if (!l.closest('.kit-card')) api.textNode(`label-${slug(l.textContent)}`, l); });
// The invitation card (`.kit-card`).
qa('.mk-body .kit-card').forEach((card, n) => {
  const k = `card-${n}`;
  api.box(k, card);
  api.box(`${k}-tile`, q('.mk-ftile', card));
  api.icon(`${k}-icon`, q('.mk-ftile svg', card));
  api.text(`${k}-label`, q('.t-label', card));
  api.text(`${k}-head`, q('.t-head', card));
  api.text(`${k}-sub`, q('.t-sub', card));
});
// A row's person (the From column): avatar and name.
qa('.mk-rows > .mk-row').forEach((row, i) => {
  const from = q(':scope > .min-w-0 > .flex', row);
  if (!from) return;
  api.text(`row-${i}-from-avatar`, q('.kit-avatar', from));
  api.text(`row-${i}-from`, q('.mk-cell', from));
});
// A grouped list (`.kit-list`): `list-<n>`, its items `item-<n>-<m>` and their parts.
qa('.kit-list').filter((list) => !inDet(list)).forEach((list, n) => {
  api.text(`list-${n}-title`, q(':scope > .kit-list-title', list));
  api.text(`list-${n}-foot`, q(':scope > .kit-list-foot', list));
  api.box(`list-${n}`, q(':scope > .kit-list-body', list));
  qa('.kit-item', list).forEach((item, m) => {
    const k = `item-${n}-${m}`;
    api.box(k, item, { selected: item.classList.contains('is-selected') });
    const tile = q('.mk-ftile', item);
    if (tile) { api.box(`${k}-tile`, tile, { thumb: tile.classList.contains('mk-ph') }); api.icon(`${k}-tile-icon`, q('svg', tile)); }
    api.icon(`${k}-icon`, q('.kit-item-icon svg', item));
    const lead = q(':scope > .kit-avatar', item);
    if (lead) api.text(`${k}-person`, lead);
    api.text(`${k}-title`, q('.kit-item-title', item));
    api.text(`${k}-sub`, q('.kit-item-sub', item));
    api.text(`${k}-value`, q('.kit-item-value', item));
    qa('.kit-avatars .kit-avatar', item).forEach((a, j) => api.text(`${k}-avatar-${j}`, a));
    const pill = q('.kit-pill', item);
    if (pill) { api.box(`${k}-pill`, pill); api.box(`${k}-pill-dot`, q('.kit-dot', pill)); api.textNode(`${k}-pill-label`, pill); }
    const link = q('.mk-link', item);
    if (link) { api.icon(`${k}-link-icon`, q('svg', link)); api.textNode(`${k}-link`, link); }
    api.icon(`${k}-more`, q(':scope > .mk-mac svg', item));
    api.icon(`${k}-chev`, q('.kit-item-chev svg', item));
    const pop = q('.mk-pop2', item);
    if (pop) { api.box(`${k}-pop`, pop); api.textNode(`${k}-pop-label`, pop); api.icon(`${k}-pop-caret`, q('svg', pop)); }
  });
});
// An empty state (`.kit-empty`).
const empty = q('.kit-empty');
if (empty) {
  api.box('empty-tile', q('.kit-tile', empty));
  api.icon('empty-icon', q('.kit-tile svg', empty));
  api.text('empty-title', q('.t-title', empty));
  api.text('empty-sub', q('.t-sub', empty));
}
// A context menu (`.wn-menu`): `menu`, items `mi-<its words>` (+ `-label`, `-sc`, `-sc-icon`), separators `msep-<n>`.
qa('.wn-menu').forEach((menu) => {
  api.box('menu', menu);
  qa('.wn-mi', menu).forEach((mi) => {
    const k = `mi-${slug(q('.lb', mi).textContent)}`;
    api.box(k, mi, { hot: mi.classList.contains('is-hot') });
    api.text(`${k}-label`, q('.lb', mi));
    const sc = q('.sc', mi);
    if (sc && q('svg', sc)) api.icon(`${k}-sc-icon`, q('svg', sc)); else if (sc) api.text(`${k}-sc`, sc);
  });
  qa('.wn-msep', menu).forEach((sep, n) => api.box(`msep-${n}`, sep));
});
// The emptying note (`.mk-banner`) and its bar.
const mkBanner = q('.mk-banner');
if (mkBanner) {
  api.box('banner', mkBanner);
  api.text('banner-text', q(':scope > span', mkBanner));
  api.box('banner-bar', q('.mk-bar', mkBanner));
  api.box('banner-bar-fill', q('.mk-bar i', mkBanner));
}
} // not a settings frame, not an account window

// ---- Settings, inside the window (`.mk-set`: lane f-settings, src/frames/settings) ----
// One scheme for every pane: lists and their rows by order (`list-<L>`, `item-<L>-<I>`), every
// button, banner, card and menu by order. A pane's own drawings (charts, trees) have their own lines.
const set = q('.mk-set');
if (set) {
  qa('.mk-side-title', side).forEach((tt) => api.textNode(`side-title-${slug(tt.textContent)}`, tt));
  const sfoot = q('.mk-side-foot', side);
  if (sfoot) { api.box('side-foot-box', sfoot); api.textNode('side-foot', sfoot); }
  const main = q('.mk-set-main', set);
  api.box('set-main', main);
  const crumb = q('.mk-set-col > .t-sub .kit-go > span', main);
  if (crumb) { api.icon('crumb-icon', q('svg', crumb)); api.textNode('crumb', crumb); }
  api.text('set-title', q('.mk-set-h .t-large', main));
  api.text('set-sub', q('.mk-set-h .t-sub', main));
  qa('.kit-list', set).forEach((list, L) => {
    const title = q(':scope > .kit-list-title', list);
    if (title) api.textNode(`list-${L}-title`, title);
    api.box(`list-${L}-body`, q(':scope > .kit-list-body', list));
    const foot = q(':scope > .kit-list-foot', list);
    if (foot) api.textNode(`list-${L}-foot`, foot);
    qa('.kit-item', list).forEach((item, I) => {
      const k = `item-${L}-${I}`;
      api.box(k, item, { selected: item.classList.contains('is-selected') });
      api.icon(`${k}-icon`, q('.kit-item-icon svg', item));
      api.text(`${k}-title`, q('.kit-item-title', item));
      api.text(`${k}-sub`, q('.kit-item-sub', item));
      api.text(`${k}-value`, q('.kit-item-value', item));
      api.icon(`${k}-chev`, q('.kit-item-chev svg', item));
      api.icon(`${k}-check`, q('.kit-item-check svg', item));
      api.icon(`${k}-ext`, q(':scope > svg', item));
      api.text(`${k}-time`, q('span.font-mono.t-cap', item));
      const pop = q('.mk-pop2', item);
      if (pop) { api.box(`${k}-pop`, pop); api.textNode(`${k}-pop-label`, pop); api.icon(`${k}-pop-caret`, q('svg', pop)); }
      const sw = q('[role="switch"] > span', item);
      if (sw) { api.box(`${k}-switch`, sw, { checked: sw.parentElement.getAttribute('aria-checked') === 'true' }); api.box(`${k}-knob`, q('span', sw)); }
    });
  });
  qa('.kit-btn', set).forEach((b, i) => {
    api.box(`btn-${i}`, b);
    api.icon(`btn-${i}-icon`, q('svg', b));
    const lab = q('span', b);
    if (lab) api.text(`btn-${i}-label`, lab); else api.textNode(`btn-${i}-label`, b);
  });
  qa('.kit-banner', set).forEach((bn, i) => {
    api.box(`banner-${i}`, bn);
    api.icon(`banner-${i}-icon`, q('.kit-banner-icon svg', bn));
    api.text(`banner-${i}-title`, q('.kit-banner-text > b', bn));
    api.text(`banner-${i}-text`, q('.kit-banner-text > span', bn));
  });
  qa('.kit-card', set).forEach((c, i) => {
    api.box(`card-${i}`, c);
    api.icon(`card-${i}-mark`, q(':scope > svg[viewBox="0 0 512 512"]', c), { mark: true });
    api.text(`card-${i}-head`, q('.t-head', c));
    api.text(`card-${i}-sub`, q('.t-sub', c));
  });
  // A notice inside a pane (`.mk-banner`), a toast over it, a toned tile (a state's glyph on its tint).
  qa('.mk-banner', set).forEach((bn, i) => {
    api.box(`note-${i}`, bn);
    api.icon(`note-${i}-icon`, q(':scope > svg', bn));
    api.text(`note-${i}-text`, q('.tx', bn));
  });
  qa('.kit-toast', set).forEach((to, i) => {
    api.box(`toast-${i}`, to);
    api.icon(`toast-${i}-icon`, q('svg', to));
    api.text(`toast-${i}-text`, q('span', to));
  });
  qa('.kit-pill', set).forEach((pl, i) => {
    api.box(`pill-${i}`, pl);
    api.box(`pill-${i}-dot`, q('.kit-dot', pl));
    api.textNode(`pill-${i}-label`, pl);
  });
  qa('.kit-tile', set).forEach((ti, i) => {
    api.box(`tile-${i}`, ti);
    api.icon(`tile-${i}-icon`, q('svg', ti));
  });
  qa('.wn-menu', set).forEach((menu, m) => {
    api.box(`menu-${m}`, menu);
    qa('.wn-mi', menu).forEach((mi, i) => {
      api.box(`menu-${m}-item-${i}`, mi, { hot: mi.classList.contains('is-hot') });
      api.icon(`menu-${m}-item-${i}-check`, q('.ck svg', mi));
      api.text(`menu-${m}-item-${i}-label`, q('.lb', mi));
    });
  });
  // A dialog over the pane (`.wn-dlg` on `.mk-scrim`).
  api.box('scrim', q('.mk-scrim', set));
  const dlg = q('.wn-dlg', set);
  if (dlg) {
    api.box('dlg', dlg);
    api.text('dlg-title', q('.wn-dlg-head .t-title', dlg));
    api.text('dlg-sub', q('.wn-dlg-head .t-sub', dlg));
    qa('.wn-dlg-body > p', dlg).forEach((p, i) => api.text(`dlg-text-${i}`, p));
    qa('.wn-dlg-body li', dlg).forEach((li, i) => api.text(`dlg-li-${i}`, li));
    api.box('dlg-foot', q('.wn-dlg-foot', dlg));
  }
}
