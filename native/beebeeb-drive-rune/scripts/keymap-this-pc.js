// keymap-this-pc.js: the gate keys of Sync history and Available offline (DW-1301 to DW-1306,
// DW-0428, DW-0715, DW-0120). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
// By the board's class names, counted in document order: the native side counts the same way.
{
  const tb = q('.mk-tb');
  // ---- The filter (`.kit-seg`): All · This PC · Other devices · Problems ----
  const seg = tb && q(':scope > .kit-seg', tb);
  if (seg) {
    api.box('filter', seg);
    qa(':scope > span', seg).forEach((s, i) => {
      api.box(`filter-${i}`, s, { on: s.classList.contains('on') });
      api.textNode(`filter-${i}-label`, s);
    });
  }
  // ---- An outlined button in the toolbar: Remove all from this PC ----
  const tbBtn = tb && q('button.kit-btn', tb);
  if (tbBtn) { api.box('remove-all', tbBtn); api.text('remove-all-label', q('span', tbBtn)); }
  // ---- The search field with words in it (DW-1304): the caret and the clear glyph ----
  const search = tb && q('.mk-search', tb);
  if (search) {
    api.box('search-caret', q('.kit-caret', search));
    api.icon('search-clear', q('.mk-mac svg', search));
  }

  // ---- The bar under the toolbar (`.mk-bulk`): Can't sync ----
  const bulk = q('.mk-main > .mk-bulk');
  if (bulk) {
    api.box('problems-bar', bulk);
    api.text('problems-bar-title', q(':scope > b', bulk));
    api.text('problems-bar-text', q(':scope > b + span', bulk));
    const all = q('button', bulk);
    api.box('try-all', all);
    api.icon('try-all-icon', all && q('svg', all));
    api.text('try-all-label', all && q('span', all));
  }

  // ---- The note over a list (`.mk-banner`) ----
  const note = q('.mk-main > .mk-banner');
  if (note) {
    api.box('note', note);
    api.icon('note-icon', q('svg', note));
    api.text('note-text', q('.tx', note));
  }

  // ---- The history: days (`.t-label`) and their changes ----
  const body = q('.mk-body');
  const days = body ? qa('.t-label', body).filter((l) => l.parentElement.querySelector(':scope > .grid')) : [];
  let n = 0;
  days.forEach((label, d) => {
    if (label.getBoundingClientRect().top < body.getBoundingClientRect().bottom) api.textNode(`day-${d}`, label);
    qa(':scope > .grid', label.parentElement).forEach((row) => {
      const k = `change-${n++}`;
      // A change wholly below the list's bottom edge is not on screen: nothing of it to compare.
      if (row.getBoundingClientRect().top >= body.getBoundingClientRect().bottom) return;
      const cells = [...row.children];
      api.box(k, row);
      api.box(`${k}-tile`, cells[0], { thumb: cells[0].classList.contains('mk-ph') });
      api.icon(`${k}-icon`, q('svg', cells[0]));
      api.text(`${k}-name`, cells[1].children[0]);
      api.text(`${k}-sub`, cells[1].children[1]);
      api.icon(`${k}-device-icon`, q('svg', cells[2]));
      api.text(`${k}-device`, q('span', cells[2]));
      const retry = q('button', cells[3]);
      if (retry) {
        api.box(`${k}-retry`, retry);
        api.icon(`${k}-retry-icon`, q('svg', retry));
        api.text(`${k}-retry-label`, q('span', retry));
      }
      api.text(`${k}-time`, cells[4]);
    });
  });

  // ---- Nothing to list (`.kit-empty`) ----
  const empty = body && q('.kit-empty', body);
  if (empty && !q('button', empty)) {
    api.box('empty', empty);
    api.box('empty-tile', q('.kit-tile', empty));
    api.icon('empty-icon', q('.kit-tile svg', empty));
    api.text('empty-title', q('.t-title', empty));
    api.text('empty-text', q('.t-sub', empty));
  }

  // ---- A row's menu (`.wn-menu`, DW-1306) ----
  const menu = q('.wn-menu');
  if (menu) {
    api.box('menu', menu);
    qa('.wn-mi', menu).forEach((it, i) => {
      api.box(`menu-item-${i}`, it, { hot: it.classList.contains('is-hot') });
      api.text(`menu-item-${i}-label`, q('.lb', it));
      if (q('.sc', it)) api.textNode(`menu-item-${i}-shortcut`, q('.sc', it));
    });
    qa('.wn-msep', menu).forEach((sep, i) => api.box(`menu-sep-${i}`, sep));
  }
}
