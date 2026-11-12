// keymap-files.js: the keys the Files frames add (menus, toasts). Same rules as keymap.js: by the board's class names.
// Only in a Files frame (`groupOf`, keymap.js): Shared, Trash, settings and the account windows key
// their own menus and empty states under their own names (integration I2).
if (groupOf === 'files') {

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
  if (add) { // an empty state may have no button
    api.box('empty-add', add);
    api.icon('empty-add-icon', q('svg', add));
    api.text('empty-add-label', q('span', add));
  }
  api.text('empty-hint', q('.t-cap', empty));
}
} // groupOf === 'files'

// ==== Lane f-files-rest: the dialogs and the name field of the Files frames ====
if (groupOf === 'files') {

// ---- A dialog on the window's scrim (`.mk-scrim` + `.wn-dlg`): New folder (DW-0417), Move to (DW-0409) ----
// Not the commands and shortcuts dialog (`.wn-cmds`): keymap-shortcuts.js keys that one (`sc-*`).
const dlg = q('.wn-dlg .wn-cmds') ? null : q('.wn-dlg');
if (dlg) {
  api.box('scrim', q('.mk-scrim'));
  api.box('dlg', dlg);
  api.text('dlg-title', q('.wn-dlg-head .t-title', dlg));
  api.text('dlg-sub', q('.wn-dlg-head .t-sub', dlg));
  api.box('dlg-foot', q('.wn-dlg-foot', dlg));
  // A button (`.kit-btn`), numbered in the dialog's own order.
  qa('.kit-btn', dlg).forEach((b, i) => {
    api.box(`btn-${i}`, b);
    api.icon(`btn-${i}-icon`, q('svg', b));
    api.text(`btn-${i}-label`, q('span', b));
  });
  // A field without a label above it (`.kit-fieldwrap` > `.kit-field`).
  qa('.kit-fieldwrap', dlg).forEach((w, i) => {
    const f = q('.kit-field', w);
    api.box(`field-${i}`, f, { focus: f.classList.contains('is-focus') });
    api.text(`field-${i}-value`, q('.kit-field-value > span', f));
    api.box(`field-${i}-caret`, q('.kit-caret', f));
    api.text(`field-${i}-msg`, q('.kit-field-msg', w));
  });
  // The path line of Move to: a folder mark and the folders, the one shown last.
  const path = q('.wn-dlg-body > .t-sub', dlg);
  if (path) {
    api.icon('dlg-path-icon', q(':scope > svg', path));
    qa(':scope > span', path).forEach((c, i) => api.text(`dlg-path-${i}`, c));
  }
  // The folders to choose from (`.kit-list` > `.kit-item`).
  qa('.kit-list', dlg).forEach((list, L) => {
    api.box(`list-${L}-body`, q(':scope > .kit-list-body', list));
    qa('.kit-item', list).forEach((item, I) => {
      const k = `item-${L}-${I}`;
      api.box(k, item, { selected: item.classList.contains('is-selected') });
      api.icon(`${k}-icon`, q('.kit-item-icon svg', item));
      api.text(`${k}-title`, q('.kit-item-title', item));
      api.text(`${k}-value`, q('.kit-item-value', item));
      api.icon(`${k}-chev`, q('.kit-item-chev svg', item));
    });
  });
}

// ---- A row's name as a field (DW-0410), and the reason it cannot be given (`.mk-tip`, DW-0410b) ----
const ren = q('.mk-rows .mk-name > span:has(> .kit-caret)');
if (ren) {
  api.box('rename-field', ren);
  api.text('rename-text', ren);
  api.box('rename-caret', q('.kit-caret', ren));
}
const tip = q('.mk-tip');
if (tip) {
  api.box('tip', tip);
  api.icon('tip-icon', q('svg', tip));
  api.textNode('tip-text', tip);
}
} // groupOf === 'files'
