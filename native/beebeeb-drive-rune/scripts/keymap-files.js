// keymap-files.js: the keys the Files frames add (menus, toasts). Same rules as keymap.js: by the board's class names.

// ---- Menus (`.wn-menu`): the file menu, Sort, New, the list's background menu and its submenu ----
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

// ---- Toast (`.mk-toast > .kit-toast`) ----
const toast = q('.mk-toast .kit-toast');
if (toast) {
  api.box('toast', toast);
  api.icon('toast-icon', q('svg', toast));
  const span = q(':scope > span', toast);
  if (span && q('b', span)) {
    api.textNode('toast-text', span);
    api.box('toast-action', q('b', span));
    api.textNode('toast-action-text', q('b', span));
  } else api.text('toast-text', span);
}

// ---- The path in the toolbar's title (`.mk-crumbs`): Files > Lisbon trip ----
const crumbs = q('.mk-title .mk-crumbs');
if (crumbs) {
  qa(':scope > i', crumbs).forEach((c, i) => api.text(`crumb-${i}`, c));
  qa(':scope > svg', crumbs).forEach((c, i) => api.icon(`crumb-sep-${i}`, c));
  api.text('crumb-here', q(':scope > span', crumbs));
}

// ---- An empty folder (`.kit-empty`) ----
const empty = q('.mk-body .kit-empty');
if (empty) {
  api.box('empty', empty);
  api.box('empty-tile', q('.kit-tile', empty));
  api.icon('empty-icon', q('.kit-tile svg', empty));
  api.text('empty-title', q('.t-title', empty));
  api.text('empty-text', q('.t-sub', empty));
  const add = q('button', empty);
  api.box('empty-add', add);
  api.icon('empty-add-icon', q('svg', add));
  api.text('empty-add-label', q('span', add));
  api.text('empty-hint', q('.t-cap', empty));
}
