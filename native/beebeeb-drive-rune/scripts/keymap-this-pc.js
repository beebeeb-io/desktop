// keymap-this-pc.js: the gate keys of Sync history and Available offline (DW-1301 to DW-1306,
// DW-0428, DW-0715, DW-0120). Runs after keymap.js, in the same scope (`win`, `api`, `q`, `qa`).
// By the board's class names, counted in document order: the native side counts the same way.
{
  const tb = q('.mk-tb');
  // The filter (`.kit-seg`) is keymap.js's `seg-0`, its tabs `seg-0-<words>`; a `.kit-btn` its `btn-<words>`.
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
  }

  // ---- The note over a list (`.mk-banner`) ----
  const note = q('.mk-main > .mk-banner');
  if (note) {
    // The box and its words are keymap.js's `banner`, `banner-text`.
    api.icon('note-icon', q('svg', note));
  }

  // ---- The history: days (`.t-label`) and their changes ----
  const body = q('.mk-body');
  const days = body ? qa('.t-label', body).filter((l) => l.parentElement.querySelector(':scope > .grid')) : [];
  let n = 0;
  days.forEach((label) => {
    // The day's own words are keymap.js's `label-<words>`.
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
      api.text(`${k}-time`, cells[4]);
    });
  });

  // ---- Nothing to list (`.kit-empty`) ----
  const empty = body && q('.kit-empty', body);
  if (empty && !q('button', empty)) {
    api.box('empty', empty);
  }

  // A row's menu (`.wn-menu`, DW-1306) is keymap.js's `menu`, `mi-<words>`, `msep-<n>`.
}
