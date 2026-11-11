// keymap.d/files.js: the keys the Files frames add (menus, toasts). Same rules as keymap.js: by the board's class names.

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
