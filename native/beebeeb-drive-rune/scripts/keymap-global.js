// keymap-global.js: the gate keys of the global elements (the board's "Global elements on
// Windows": the banner under the toolbar, the loading strip, the crop's fade). Runs after
// keymap.js, in the same scope (`win`, `api`, `q`, `qa`). Keys are class names; boxes carry
// `css` (radius, padding, gap: the gate ignores it, the builder reads it).
{
  const gcss = (el) => {
    const cs = getComputedStyle(el);
    const out = {};
    if (cs.borderRadius !== '0px') out.radius = cs.borderRadius;
    if (cs.boxShadow !== 'none') out.shadow = cs.boxShadow;
    if (parseFloat(cs.borderTopWidth)) out.border = `${cs.borderTopWidth} ${cs.borderTopColor}`;
    if (cs.padding !== '0px') out.pad = cs.padding;
    if (cs.margin !== '0px') out.margin = cs.margin;
    if (cs.gap !== 'normal') out.gap = cs.gap;
    if (cs.backgroundImage !== 'none') out.image = cs.backgroundImage;
    return { css: out };
  };
  const banner = q('.mk-tb + .mk-banner');
  if (banner) {
    api.box('banner', banner, { ...gcss(banner), role: banner.getAttribute('role') || '' });
    api.icon('banner-icon', q(':scope > svg', banner));
    const tx = q(':scope > .tx', banner);
    if (tx) {
      api.text('banner-text', tx, { lh: getComputedStyle(tx).lineHeight });
      // The figures inside the sentence are set in the machine face: one key per run.
      qa('.font-mono', tx).forEach((m, i) => api.text(`banner-figure-${i}`, m));
    }
    const act = q(':scope > .kit-go > button', banner);
    if (act) { api.box('banner-action', act, gcss(act)); api.text('banner-action-label', q('span', act)); }
    const x = q(':scope > .x', banner);
    if (x) { api.box('banner-dismiss', x, { name: x.getAttribute('aria-label') }); api.icon('banner-dismiss-icon', q('svg', x)); }
    // The loading strip (DW-0118): a banner with a title line, a bar and a line of states.
    const head = q(':scope > .flex > b', banner);
    if (head) {
      api.text('strip-title', head);
      const count = q(':scope > .flex > .ml-auto', banner);
      api.text('strip-count', count);
      qa('.font-mono', count).forEach((m, i) => api.text(`strip-figure-${i}`, m));
      const bar = q(':scope > .mk-bar', banner);
      api.box('strip-bar', bar, gcss(bar));
      api.box('strip-bar-fill', q('i', bar));
      api.text('strip-detail', q(':scope > div.t-cap', banner));
    }
  }
  // The toast (DW-0107 to DW-0109): a pill at the bottom of the content area.
  const toast = q('.mk-toast > .kit-toast');
  if (toast) {
    api.box('toast', toast, { ...gcss(toast), tone: (toast.className.match(/tone-(\w+)/) || [])[1] || '' });
    api.icon('toast-icon', q(':scope > svg', toast));
    const span = q(':scope > span', toast);
    api.textNode('toast-text', span, { lh: getComputedStyle(span).lineHeight });
    const act = q('b', span);
    if (act) { api.box('toast-action', act, gcss(act)); api.textNode('toast-action-label', act); }
  }
  // The empty state (DW-0114) and the same block inside a preview that cannot open (DW-0121).
  const empty = q('.kit-empty');
  if (empty) {
    api.box('empty', empty, gcss(empty));
    const tile = q(':scope > .kit-tile', empty);
    api.box('empty-tile', tile, gcss(tile));
    api.icon('empty-icon', q('svg', tile));
    api.text('empty-title', q(':scope > .t-title', empty), { lh: getComputedStyle(q(':scope > .t-title', empty)).lineHeight });
    const sub = q(':scope > .t-sub', empty);
    api.text('empty-sub', sub, { lh: getComputedStyle(sub).lineHeight, align: getComputedStyle(sub).textAlign });
    const btn = q('button', empty);
    if (btn) {
      api.box('empty-action', btn, gcss(btn));
      api.icon('empty-action-icon', q('svg', btn));
      api.text('empty-action-label', q('span', btn));
    }
    api.text('empty-hint', q('.t-cap', empty));
  }
  // The preview panel in front of the window (DW-0121): the scrim, the card, its title bar.
  const scrim = q('.mk-scrim');
  if (scrim) {
    api.box('scrim', scrim);
    const card = scrim.nextElementSibling;
    api.box('preview-card', card, gcss(card));
    const bar = card.firstElementChild;
    api.box('preview-bar', bar, gcss(bar));
    const close = q('.mk-tbtn', bar);
    api.box('preview-close', close, gcss(close));
    api.icon('preview-close-icon', q('svg', close));
    api.text('preview-title', q('.mk-title b', bar));
  }
  // The grid (DW-0119): a tile, a name and a line under it per item; a plain tile until its thumbnail arrives.
  const grid = q('.mk-gridview');
  if (grid) {
    api.box('grid', grid, gcss(grid));
    qa(':scope > .mk-gcell', grid).forEach((cell, i) => {
      const k = `grid-${i}`;
      api.box(k, cell, gcss(cell));
      const thumb = q('.mk-gthumb', cell);
      api.box(`${k}-thumb`, thumb, { ...gcss(thumb), plain: thumb.classList.contains('is-plain') });
      api.icon(`${k}-icon`, q(':scope > svg', thumb));
      const badge = q('.mk-gbadge', thumb);
      if (badge) { api.box(`${k}-badge`, badge, gcss(badge)); api.icon(`${k}-badge-icon`, q('svg', badge)); api.textNode(`${k}-badge-text`, badge); }
      api.text(`${k}-name`, q('.mk-gname', cell));
      api.text(`${k}-sub`, q('.mk-gsub', cell));
    });
    const crumbs = q('.mk-title .mk-crumbs');
    if (crumbs) {
      api.text('crumb-parent', q(':scope > i', crumbs));
      api.icon('crumb-sep', q(':scope > svg', crumbs));
      api.text('crumb-current', q(':scope > span', crumbs));
      api.text('crumb-count', q('.mk-title > span:not(.mk-crumbs)'));
    }
  }
  const fade = q(':scope > .mk-cropfade, .mk-cropfade');
  if (fade) api.box('crop-fade', fade, gcss(fade));
}
