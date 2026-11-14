// keymap-mail.js: the gate keys of Mail Backup inside Backups (DW-0740, DW-0740b, DW-1401 to
// DW-1406, DW-0743, DW-0743b) beyond what keymap.js and keymap-backups.js key for every settings
// pane. Runs after them, in the same scope (`win`, `api`, `q`, `qa`). Only where the settings
// sidebar has Backups selected AND the pane has Mail Backup's head (a tile beside a `.t-title`,
// not a card): no
// other frame's golden changes. By the board's class names, in document order.
{
  const setM = q('.mk-set');
  const onM = setM && qa('.mk-side-item', setM).find((i) => i.classList.contains('on'));
  const headM = setM && q('.mk-set-col > .flex.items-center.gap-3', setM);
  if (onM && onM.textContent.trim() === 'Backups' && headM && !headM.classList.contains('kit-card') && q(':scope > .kit-tile', headM) && q('.t-title', headM)) {
    api.text('mail-title', q('.t-title', headM));
    api.text('mail-sub', q('.t-sub', headM));
    // What Mail Backup is, under the kept folder (off).
    api.text('mail-note', q('.mk-set-col > p.t-sub', setM));
    // The first archive: how far, how long.
    const progress = q('.kit-card > .flex.justify-between.t-sub', setM);
    if (progress) {
      const spans = qa(':scope > span', progress);
      api.text('mail-count', spans[0]);
      api.text('mail-left', spans[1]);
    }
    // The account step's fields (`.kit-fieldwrap` > `.kit-field`).
    qa('.wn-dlg .kit-fieldwrap', setM).filter((w) => q('.kit-field', w)).forEach((w, i) => {
      const f = q('.kit-field', w);
      api.text(`field-${i}-label`, q('.kit-field-outer', w));
      api.box(`field-${i}`, f, { focus: f.classList.contains('is-focus'), error: f.classList.contains('is-error') });
      const value = q('.kit-field-value', f);
      if (value && value.textContent.trim()) api.textNode(`field-${i}-value`, value, { secure: value.classList.contains('is-secure') });
      api.box(`field-${i}-caret`, q('.kit-caret', f));
      api.text(`field-${i}-msg`, q('.kit-field-msg', w));
    });
  }
}
