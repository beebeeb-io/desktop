// keymap-settings.js: the keys of Drive's own settings panes (lane f-settings-rest,
// src/frames/settings: Storage, Storage insights, Sync and File Explorer, Thumbnails and
// previews, Bandwidth, Diagnostics). Read after keymap.js, in the same scope: `set` is the
// settings view (`.mk-set`), and keymap.js has keyed its lists, rows, buttons, cards, tiles,
// menus and dialog already. Everything here is by the board's class names, numbered in order.
if (set) {
  // Measures: a meter and its parts, a share bar, a slider, a chip, a skeleton.
  qa('.kit-meter', set).forEach((m, i) => {
    api.box(`meter-${i}`, m);
    qa(':scope > i', m).forEach((part, j) => api.box(`meter-${i}-part-${j}`, part));
  });
  qa('.mk-insbar', set).forEach((b, i) => {
    api.box(`insbar-${i}`, b);
    api.box(`insbar-${i}-fill`, q('i', b));
    api.text(`insbar-${i}-name`, q('.t-sub', b.previousElementSibling));
  });
  const slider = q('.mk-slider', set);
  if (slider) {
    api.box('slider', slider);
    api.box('slider-track', q('i', slider));
    api.box('slider-fill', q('u', slider));
    api.box('slider-knob', q('b', slider));
    const ends = qa('span', slider.nextElementSibling);
    api.text('slider-min', ends[0]);
    api.text('slider-max', ends[1]);
  }
  qa('.kit-chip', set).forEach((c, i) => {
    api.box(`chip-${i}`, c);
    api.icon(`chip-${i}-icon`, q('svg', c));
    api.textNode(`chip-${i}-label`, c);
  });
  qa('.mk-skel', set).forEach((sk, i) => api.box(`skel-${i}`, sk));
  qa('.mk-fig-v', set).forEach((f, i) => api.text(`fig-${i}`, f));
  // A caption line in the pane's column, and the captions inside a card (a caption that
  // holds machine values is keyed per value).
  qa('.mk-set-col > p.t-cap', set).forEach((p, i) => api.text(`cap-${i}`, p));
  qa('.kit-card', set).forEach((c, i) => {
    qa('.t-cap, .t-cap .font-mono', c)
      .filter((e) => !e.children.length && e.textContent.trim() && !e.closest('.mk-lrow'))
      .forEach((e, j) => api.text(`card-${i}-cap-${j}`, e));
  });
  // Storage: the account's use above its meter.
  const use = q('.mk-meter2', set);
  if (use) {
    api.text('usage-used', q('.t-title', use.parentElement));
    api.text('usage-total', q('.t-sub', use.parentElement));
  }
  // Storage insights: the largest files.
  qa('.mk-lrow', set).forEach((r, i) => {
    if (r.classList.contains('t-cap')) { api.text('lrow-more', q('span', r)); return; }
    api.box(`lrow-${i}`, r);
    api.text(`lrow-${i}-path`, q('.p', r));
    api.text(`lrow-${i}-size`, q('.s', r));
  });
  // A limit dialog's figure.
  api.text('dlg-figure', q('.wn-dlg-body > .t-large', set));
}
