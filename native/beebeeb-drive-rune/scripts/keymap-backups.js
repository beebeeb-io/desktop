// keymap-backups.js: the gate keys of Backups (DW-0737, DW-1411, DW-1410, DW-0739, DW-0742,
// DW-0741) beyond what keymap.js keys for every settings pane. Runs after keymap.js, in the
// same scope (`win`, `api`, `q`, `qa`). Only where the settings sidebar has Backups selected:
// no other pane's golden changes. By the board's class names, in document order.
{
  const setB = q('.mk-set');
  const onB = setB && qa('.mk-side-item', setB).find((i) => i.classList.contains('on'));
  if (onB && onB.textContent.trim() === 'Backups') {
    qa('.kit-card', setB).forEach((c, i) => {
      // The card that is one link ("Couldn't back up 3 items"): its glyph, its words, its chevron.
      api.icon(`card-${i}-icon`, q(':scope > .mk-mac svg', c));
      api.text(`card-${i}-text`, q(':scope > .t-body', c));
      api.icon(`card-${i}-chev`, q(':scope > svg', c));
      // Progress in a card (`.mk-bar`).
      const bar = q('.mk-bar', c);
      if (bar) { api.box(`card-${i}-bar`, bar); api.box(`card-${i}-bar-fill`, q('i', bar)); }
    });
    // The one-time code (`.kit-fieldwrap` in a dialog): its label, the six boxes, the caret.
    const wrap = q('.wn-dlg .kit-fieldwrap', setB);
    if (wrap) {
      api.text('code-label', q('.kit-field-outer', wrap));
      qa(':scope > div > span', wrap).forEach((b, i) => api.box(`code-${i}`, b));
      api.box('code-caret', q('.kit-caret', wrap));
    }
  }
}
