// keymap-photos.js: the gate keys of the Photos group (DW-0601 to DW-0611, DW-0116). Runs after
// keymap.js, in the same scope (`win`, `api`, `q`, `qa`), and only in a frame whose sidebar has
// Photos on. Every key starts with `ph-`, so no other group's keymap can name the same element
// twice. Keys are class names and running numbers in document order; the native side counts the
// same way. Boxes carry `css` (radius, shadow, border, padding, gap, opacity): the gate ignores
// it, the person building the frame reads the measurements from the golden.
const phOn = qa('.mk-side-item.on .mk-side-label').some((l) => l.textContent.trim() === 'Photos');
if (phOn) {
  const css = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (parseFloat(cs.borderTopWidth) || parseFloat(cs.borderRightWidth) || parseFloat(cs.borderBottomWidth)) out.border = `${cs.borderTopWidth} ${cs.borderRightWidth} ${cs.borderBottomWidth} ${cs.borderLeftWidth} ${cs.borderTopColor}`;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (cs.opacity !== '1') out.opacity = cs.opacity;
    if (cs.backgroundImage !== 'none') out.image = cs.backgroundImage;
    if (cs.backdropFilter && cs.backdropFilter !== 'none') out.backdrop = cs.backdropFilter;
    return { css: out };
  };
  const box = (key, el, extra = {}) => { if (el) api.box(key, el, { ...css(el), ...extra }); };
  const text = (key, el) => { if (el) api.text(key, el, { lh: getComputedStyle(el).lineHeight }); };
  const photo = (el) => (el.className.match(/mk-ph(\d)/) || [])[1];
  const mkTb = q('.mk-tb');

  // ---- Toolbar: Photos across, Photo sources, Select; in the viewer the two pills and the ⋯ button ----
  [['ph-across', 'Photos across'], ['ph-sources', 'Photo sources'], ['ph-select', 'Select']].forEach(([k, title]) => {
    const b = q(`.mk-tbtn[title="${title}"]`, mkTb);
    if (!b) return;
    box(k, b, { on: b.classList.contains('on') });
    const s = qa('svg', b);
    api.icon(`${k}-icon`, s[0]);
    if (s[1]) api.icon(`${k}-caret`, s[1]);
  });
  const seg = q('.kit-seg', mkTb);
  if (seg) {
    box('ph-seg', seg);
    qa(':scope > span', seg).forEach((tab, i) => {
      box(`ph-seg-${i}`, tab, { on: tab.classList.contains('on') });
      api.textNode(`ph-seg-${i}-label`, tab);
    });
  }
  const more = q('.kit-go > .mk-tbtn:not([title])', mkTb);
  if (more) { box('ph-more', more); api.icon('ph-more-icon', q('svg', more)); }

  // ---- The bar under the toolbar: source pills, the devices' backup status ----
  const chips = qa('.mk-body .kit-chip');
  if (chips.length) {
    box('ph-head', chips[0].parentElement.parentElement);
    chips.forEach((c, i) => { box(`ph-pill-${i}`, c, { on: c.classList.contains('is-active') }); api.textNode(`ph-pill-${i}-label`, c); });
    const status = q('.mk-body .ml-auto.t-sub');
    if (status) {
      api.icon('ph-backup-icon', q('svg', status));
      text('ph-backup-strong', q('b', status));
      api.textNode('ph-backup-rest', q(':scope > span', status));
    }
  }

  // ---- The selection bar (`.mk-bulk`) ----
  const bulk = q('.mk-bulk');
  if (bulk) {
    box('ph-bulk', bulk);
    text('ph-bulk-count', q(':scope > b', bulk));
    text('ph-bulk-all', q(':scope > .underline', bulk));
    qa('.kit-btn', bulk).forEach((b, i) => {
      box(`ph-bulk-btn-${i}`, b);
      api.icon(`ph-bulk-btn-${i}-icon`, q('svg', b));
      text(`ph-bulk-btn-${i}-label`, q('span', b));
    });
  }

  // ---- The days and their tiles ----
  qa('.mk-body .grid.gap-\\[3px\\]').forEach((grid, d) => {
    api.textNode(`ph-day-${d}`, grid.previousElementSibling);
    qa(':scope > .mk-ph', grid).forEach((tile, i) => {
      const k = `ph-tile-${d}-${i}`;
      box(k, tile, { photo: photo(tile), selected: /inset/.test(tile.style.boxShadow) });
      const badge = q('.mk-gbadge', tile);
      if (badge) {
        box(`${k}-badge`, badge);
        api.icon(`${k}-badge-icon`, q('svg', badge));
        if (badge.textContent.trim()) api.textNode(`${k}-badge-label`, badge);
      }
      const check = q(':scope > .rounded-full', tile);
      if (check) { box(`${k}-check`, check, { checked: !!q('svg', check) }); api.icon(`${k}-check-icon`, q('svg', check)); }
    });
  });

  // ---- Empty (`.kit-empty`) ----
  const empty = q('.mk-body .kit-empty');
  if (empty) {
    box('ph-empty-tile', q('.kit-tile', empty));
    api.icon('ph-empty-icon', q('.kit-tile svg', empty));
    text('ph-empty-title', q('.t-title', empty));
    text('ph-empty-sub', q('.t-sub', empty));
  }

  // ---- The viewer: the stage, the photo, the strip, the four actions, the message ----
  const stage = q('.mk-body > .relative');
  if (stage) {
    box('ph-stage', stage);
    const bar = q(':scope > span.block', stage);
    if (bar) { box('ph-progress', bar); box('ph-progress-fill', q('i', bar)); }
    const big = q(':scope > .grid > .mk-ph', stage);
    box('ph-photo', big, big ? { photo: photo(big) } : {});
    const player = big && q(':scope > span', big);
    if (player) {
      box('ph-player', player);
      api.icon('ph-player-icon', q(':scope > svg', player));
      qa(':scope > .t-cap', player).forEach((c, i) => text(`ph-player-text-${i}`, c));
      const track = q(':scope > .flex-1', player);
      box('ph-player-track', track);
      box('ph-player-fill', q('i', track));
    }
    qa(':scope > .gap-1 > .mk-ph', stage).forEach((th, i) => box(`ph-strip-${i}`, th, { photo: photo(th), current: th.style.opacity === '1' }));
    qa(':scope > .gap-8 .t-cap', stage).forEach((a, i) => {
      box(`ph-act-${i}`, a);
      api.icon(`ph-act-${i}-icon`, q('svg', a));
      api.textNode(`ph-act-${i}-label`, a);
    });
    const toast = q('.kit-toast', stage);
    if (toast) {
      box('ph-toast', toast);
      api.icon('ph-toast-icon', q('svg', toast));
      const span = q(':scope > span', toast);
      const act = q('b', span);
      if (act) { api.textNode('ph-toast-text', span); box('ph-toast-action', act); api.textNode('ph-toast-action-label', act); }
      else text('ph-toast-text', span);
    }
  }

  // ---- A menu (`.wn-menu`): Photos across, the ⋯ menu ----
  const menu = q('.wn-menu');
  if (menu) {
    box('ph-menu', menu);
    qa('.wn-mi', menu).forEach((mi, i) => {
      const k = `ph-menu-${i}`;
      box(k, mi, { hot: mi.classList.contains('is-hot'), head: mi.classList.contains('is-head') });
      api.icon(`${k}-check`, q('.ck svg', mi));
      text(`${k}-label`, q('.lb', mi));
      text(`${k}-sc`, q('.sc', mi));
    });
  }

  // ---- A dialog (`.wn-dlg` on `.mk-scrim`): Photo sources, Share, Details ----
  box('ph-scrim', q('.mk-scrim'));
  const dlg = q('.wn-dlg');
  if (dlg) {
    box('ph-dlg', dlg);
    text('ph-dlg-title', q('.wn-dlg-head .t-title', dlg));
    const body = q('.wn-dlg-body', dlg);
    box('ph-dlg-body', body);
    const thumb = q(':scope > .flex > .mk-ftile', body);
    if (thumb) {
      box('ph-dlg-thumb', thumb, { photo: photo(thumb) });
      text('ph-dlg-name', q(':scope > .flex .t-head', body));
      text('ph-dlg-size', q(':scope > .flex .t-cap .font-mono', body));
    }
    qa('.kit-list', body).forEach((list, n) => {
      text(`ph-list-${n}-title`, q(':scope > .kit-list-title', list));
      box(`ph-list-${n}`, q(':scope > .kit-list-body', list));
      qa('.kit-item', list).forEach((item, m) => {
        const k = `ph-item-${n}-${m}`;
        box(k, item);
        api.icon(`${k}-icon`, q('.kit-item-icon svg', item));
        text(`${k}-title`, q('.kit-item-title', item));
        text(`${k}-sub`, q('.kit-item-sub', item));
        text(`${k}-value`, q('.kit-item-value', item));
        api.icon(`${k}-chev`, q('.kit-item-chev svg', item));
        const sw = q('[role="switch"] > span', item);
        if (sw) { box(`${k}-switch`, sw, { checked: sw.parentElement.getAttribute('aria-checked') === 'true' }); box(`${k}-knob`, q('span', sw)); }
        const pop = q('.mk-pop2', item);
        if (pop) { box(`${k}-pop`, pop); api.textNode(`${k}-pop-label`, pop); api.icon(`${k}-pop-caret`, q('svg', pop)); }
      });
    });
    text('ph-dlg-note', q(':scope > p.t-cap', body));
    box('ph-dlg-foot', q('.wn-dlg-foot', dlg));
    qa('.wn-dlg-foot .kit-btn', dlg).forEach((b, i) => {
      box(`ph-dlg-btn-${i}`, b);
      api.icon(`ph-dlg-btn-${i}-icon`, q('svg', b));
      text(`ph-dlg-btn-${i}-label`, q('span', b));
    });
  }
}
