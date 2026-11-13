// keymap-search.js: the keys the Search frames add (pills, recent searches, action buttons). Runs
// after keymap.js and all other keymaps, in the same scope (`win`, `api`, `q`, `qa`, `slug`).
//
// A search frame has the Files window's toolbar with the search field focused: `.mk-search.is-focus`.
// Search draws in the content area of the Files window; the sidebar, toolbar and status bar are the
// Files window's own (keyed by keymap.js).
//
// The names are the Shared and Trash scheme (`seg-<n>`, `label-<words>`, `list-<n>`, `item-<n>-<m>`,
// `btn-<words>`), which the view code uses. Added to the board lane's goldens as a strict superset:
// every key and record those had is unchanged.
// Only in a Files frame (`groupOf`, keymap.js): Sync history (DW-1304) and a viewer's find bar have a
// focused field too, and their groups key their own lists and buttons.
const searchActive = groupOf === 'files' && !!q('.mk-search.is-focus');
if (searchActive) {

// ---- Pills (`.kit-seg`): All · Folders · Photos · Documents ----
// The Files window's view toggle uses `.mk-tbtn` inside `.mk-tgroup`, not `.kit-seg`, so `.kit-seg`
// matches only the search pills here.
qa('.kit-seg').filter((seg) => !seg.closest('.mk-det')).forEach((seg, n) => {
  api.box(`seg-${n}`, seg);
  qa(':scope > span', seg).forEach((tab) => {
    const k = `seg-${n}-${slug(tab.textContent)}`;
    api.box(k, tab, { on: tab.classList.contains('on') });
    api.textNode(`${k}-label`, tab);
  });
});

// ---- Section label (`.t-label`): "Recent" in DW-0801 ----
qa('.mk-body .t-label').forEach((l) => {
  if (!l.closest('.kit-card')) api.textNode(`label-${slug(l.textContent)}`, l);
});

// ---- Grouped list (`.kit-list`): recent searches (DW-0801) or Available offline card (DW-0802) ----
// DW-0802's list is inside `.mk-det` (the native code uses `twice()` to give the Available offline
// card both `offline-card` from keymap.js's detail pane and `list-0` from here).
qa('.kit-list').forEach((list, L) => {
  // The golden's `list-<n>` is the list's body (its fill), and an item says whether it is selected:
  // as keymap.js keys a grouped list for Shared and Trash.
  api.box(`list-${L}`, q(':scope > .kit-list-body', list));
  qa('.kit-item', list).forEach((item, I) => {
    const k = `item-${L}-${I}`;
    api.box(k, item, { selected: item.classList.contains('is-selected') });
    // A recent search has a clock icon (`.kit-item-icon svg`); the Available offline card has none.
    api.icon(`${k}-icon`, q('.kit-item-icon svg', item));
    api.text(`${k}-title`, q('.kit-item-title', item));
    api.text(`${k}-sub`, q('.kit-item-sub', item));
  });
});

// ---- Action buttons (`.kit-btn`): Share… and Version history (DW-0802 detail pane) ----
// These elements are drawn twice by the native code (once as `share-box` / `versions-box` by
// keymap.js's detail pane, once as `btn-share` / `btn-version-history` here), so the native side
// gives both names via `twice()`. The slug strips ellipsis and spaces:
// "Share…" → "share", "Version history" → "version-history".
{
  const btnSeen = {};
  qa('.kit-btn').forEach((b) => {
    const base = `btn-${slug(b.textContent)}`;
    btnSeen[base] = (btnSeen[base] || 0) + 1;
    const k = btnSeen[base] > 1 ? `${base}-${btnSeen[base]}` : base;
    api.box(k, b);
    api.icon(`${k}-icon`, q('svg', b));
    api.text(`${k}-label`, q('span', b));
  });
}

// The empty state of DW-0803 ("No results") is keyed by keymap-files.js, as an empty folder is:
// `empty`, `empty-tile`, `empty-icon`, `empty-title`, `empty-text`.

} // searchActive
