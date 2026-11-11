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
  api.textNode('this-pc-label', q('.mk-side-title', side));
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
  api.text('item-count', q('.mk-title span', tb));
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
// Everything below is keyed by class, per occurrence in document order.
// A button (`.kit-btn`): `btn-<its words>`, `-label`, `-icon`.
const btnSeen = {};
qa('.kit-btn').forEach((b) => {
  // The same words twice in one frame (New request in the toolbar and in the empty state): `-2` for the second.
  const base = `btn-${slug(b.textContent)}`;
  btnSeen[base] = (btnSeen[base] || 0) + 1;
  const k = btnSeen[base] > 1 ? `${base}-${btnSeen[base]}` : base;
  api.box(k, b);
  api.icon(`${k}-icon`, q('svg', b));
  api.text(`${k}-label`, q('span', b));
});
// A worded segmented control (`.kit-seg`): `seg-<n>`, each tab `seg-<n>-<its words>` and `-label`.
qa('.kit-seg').forEach((seg, n) => {
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
qa('.kit-list').forEach((list, n) => {
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
